//! The library: every clip and session in the output folder, with metadata
//! (cached in library.json) and thumbnails (see `thumbs.rs`). Emits
//! `library-changed` when files in the output folder change.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use ptv_common::paths;
use ptv_core::clip;
use ptv_core::config::Config;
use ptv_core::media::MediaInfo;
use ptv_core::storage;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};

use crate::thumbs::Thumbs;

/// Quiet time after the last file event before telling the window.
const DEBOUNCE: Duration = Duration::from_millis(500);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Clip,
    Session,
}

#[derive(Debug, Clone, Serialize)]
pub struct Recording {
    pub path: String,
    pub name: String,
    /// The name the user gave it (see `storage::custom_name`), if any.
    pub custom_name: Option<String>,
    /// The game's folder name (`profile::safe_name` of the game's name).
    pub game: String,
    pub kind: Kind,
    pub size: u64,
    /// Unix seconds.
    pub modified: u64,
    /// None while the file is still being written, or if it isn't readable.
    pub info: Option<MediaInfo>,
    /// Cached thumbnail, once made.
    pub thumb: Option<String>,
}

/// One cached probe, valid while the file's size and mtime are unchanged.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct CacheEntry {
    size: u64,
    modified_ms: u64,
    info: Option<MediaInfo>,
}

#[derive(Default, Serialize, Deserialize)]
struct CacheFile {
    version: u32,
    entries: HashMap<PathBuf, CacheEntry>,
}

const CACHE_VERSION: u32 = 1;

pub struct Library {
    inner: Mutex<Inner>,
    thumbs: Thumbs,
}

struct Inner {
    cache: HashMap<PathBuf, CacheEntry>,
    watching: Option<(PathBuf, RecommendedWatcher)>,
    events: std::sync::mpsc::Sender<()>,
}

pub fn init(app: &AppHandle) {
    let (tx, rx) = std::sync::mpsc::channel::<()>();
    let cache = load_cache(&paths::library_cache());
    let thumbs = Thumbs::start(app.clone());
    app.manage(Library { inner: Mutex::new(Inner { cache, watching: None, events: tx }), thumbs });
    allow_assets(app, &paths::thumbs_dir());

    let emitter = app.clone();
    std::thread::Builder::new()
        .name("library-debounce".into())
        .spawn(move || {
            while rx.recv().is_ok() {
                while rx.recv_timeout(DEBOUNCE).is_ok() {}
                let _ = emitter.emit("library-changed", ());
            }
        })
        .expect("spawn library debouncer");

    if let Ok(cfg) = Config::load_or_create(&paths::config_file()) {
        watch(app, &cfg.output_dir);
    }
}

/// Called when config.toml changes: follow a new output folder.
pub fn config_changed(app: &AppHandle, cfg: &Config) {
    if watch(app, &cfg.output_dir) {
        let _ = app.emit("library-changed", ());
    }
}

/// Lets the window load files from `dir` through the asset protocol.
fn allow_assets(app: &AppHandle, dir: &Path) {
    let _ = std::fs::create_dir_all(dir);
    if let Err(e) = app.asset_protocol_scope().allow_directory(dir, true) {
        tracing::warn!("couldn't allow {} for the asset protocol: {e}", dir.display());
    }
}

/// Watches `dir` (recursively) unless it already is; true if it changed.
fn watch(app: &AppHandle, dir: &Path) -> bool {
    let lib = app.state::<Library>();
    let mut inner = lib.inner.lock().unwrap();
    if inner.watching.as_ref().is_some_and(|(d, _)| d == dir) {
        return false;
    }
    inner.watching = None;
    allow_assets(app, dir);
    let tx = inner.events.clone();
    let watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if res.is_ok() {
            let _ = tx.send(());
        }
    });
    match watcher {
        Ok(mut w) => match w.watch(dir, RecursiveMode::Recursive) {
            Ok(()) => inner.watching = Some((dir.to_owned(), w)),
            Err(e) => tracing::warn!("couldn't watch {}: {e}", dir.display()),
        },
        Err(e) => tracing::warn!("couldn't create a file watcher: {e}"),
    }
    true
}

fn load_cache(path: &Path) -> HashMap<PathBuf, CacheEntry> {
    std::fs::read(path)
        .ok()
        .and_then(|b| serde_json::from_slice::<CacheFile>(&b).ok())
        .filter(|c| c.version == CACHE_VERSION)
        .map(|c| c.entries)
        .unwrap_or_default()
}

fn save_cache(path: &Path, entries: &HashMap<PathBuf, CacheEntry>) {
    let file = CacheFile { version: CACHE_VERSION, entries: entries.clone() };
    let Ok(json) = serde_json::to_vec(&file) else { return };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let tmp = path.with_extension("json.tmp");
    if std::fs::write(&tmp, json).is_ok() {
        let _ = std::fs::rename(&tmp, path);
    }
}

fn unix_ms(t: SystemTime) -> u64 {
    t.duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// Game folder and kind from where `storage::scan` found the file:
/// `<out>/<Game>/x.mp4` or `<out>/<Game>/Sessions/x.mp4`.
pub(crate) fn game_and_kind(path: &Path) -> (String, Kind) {
    let parent = path.parent();
    let name = |p: Option<&Path>| p.and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned());
    if name(parent).as_deref() == Some("Sessions") {
        (name(parent.and_then(|p| p.parent())).unwrap_or_default(), Kind::Session)
    } else {
        (name(parent).unwrap_or_default(), Kind::Clip)
    }
}

/// Probes `recs` (using and updating `cache`), newest first. The cache keeps
/// only entries for files that still exist.
fn build(
    recs: Vec<storage::Recording>,
    cache: &mut HashMap<PathBuf, CacheEntry>,
    probe: impl Fn(&Path) -> Option<MediaInfo>,
) -> (Vec<Recording>, bool) {
    let mut changed = false;
    let mut fresh = HashMap::with_capacity(recs.len());
    let mut out = Vec::with_capacity(recs.len());
    for r in recs {
        let modified_ms = unix_ms(r.modified);
        let entry = match cache.remove(&r.path) {
            Some(e) if e.size == r.size && e.modified_ms == modified_ms => e,
            _ => {
                changed = true;
                CacheEntry { size: r.size, modified_ms, info: probe(&r.path) }
            }
        };
        let (game, kind) = game_and_kind(&r.path);
        out.push(Recording {
            path: r.path.display().to_string(),
            name: r.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
            custom_name: storage::custom_name(&r.path).map(String::from),
            game,
            kind,
            size: r.size,
            modified: modified_ms / 1000,
            info: entry.info.clone(),
            thumb: None,
        });
        fresh.insert(r.path, entry);
    }
    changed |= !cache.is_empty(); // entries for files that are gone
    *cache = fresh;
    out.sort_by(|a, b| b.modified.cmp(&a.modified).then_with(|| b.name.cmp(&a.name)));
    (out, changed)
}

#[tauri::command]
pub async fn list_recordings(app: AppHandle) -> Result<Vec<Recording>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let cfg = Config::load_or_create(&paths::config_file()).map_err(|e| format!("{e:#}"))?;
        watch(&app, &cfg.output_dir);
        let lib = app.state::<Library>();
        let recs = storage::scan(&cfg.output_dir);
        let mut out = {
            let mut inner = lib.inner.lock().unwrap();
            let (out, changed) = build(recs, &mut inner.cache, |p| match ptv_core::media::probe(p) {
                Ok(info) => Some(info),
                Err(e) => {
                    tracing::debug!("probe {}: {e:#}", p.display());
                    None
                }
            });
            if changed {
                save_cache(&paths::library_cache(), &inner.cache);
            }
            out
        };
        lib.thumbs.attach(&mut out);
        Ok(out)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// `path`, canonicalised, if it's a recording playerstv made inside
/// `output_dir`; the only files the window may delete or open.
pub fn own_recording(path: &Path, output_dir: &Path) -> Result<PathBuf, String> {
    let refuse = || format!("{} isn't a playerstv recording in the output folder", path.display());
    let root = output_dir.canonicalize().map_err(|_| refuse())?;
    let file = path.canonicalize().map_err(|_| format!("{} no longer exists", path.display()))?;
    let name = file.file_name().and_then(|n| n.to_str()).ok_or_else(refuse)?;
    if !storage::is_recording_file(name) || !file.is_file() {
        return Err(refuse());
    }
    let rel: Vec<_> = file.strip_prefix(&root).map_err(|_| refuse())?.components().collect();
    let in_game_folder = rel.len() == 2 || (rel.len() == 3 && rel[1].as_os_str() == "Sessions");
    if !in_game_folder {
        return Err(refuse());
    }
    Ok(strip_verbatim(file))
}

/// `\\?\C:\…` → `C:\…` (Explorer and the shell APIs don't take the
/// verbatim form `canonicalize` returns).
fn strip_verbatim(p: PathBuf) -> PathBuf {
    match p.to_str().and_then(|s| s.strip_prefix(r"\\?\")) {
        Some(rest) if rest.as_bytes().get(1) == Some(&b':') => PathBuf::from(rest),
        _ => p,
    }
}

fn checked(path: &str) -> Result<PathBuf, String> {
    let cfg = Config::load_or_create(&paths::config_file()).map_err(|e| format!("{e:#}"))?;
    own_recording(Path::new(path), &cfg.output_dir)
}

/// Names a recording `name` (see `clip::named_path`); returns its new path.
#[tauri::command]
pub async fn rename_recording(app: AppHandle, path: String, name: String) -> Result<String, String> {
    let file = checked(&path)?;
    tauri::async_runtime::spawn_blocking(move || {
        let target = rename(&file, &name)?;
        // The path as listed (`file` is canonicalised), for the cache.
        let listed = Path::new(&path);
        let renamed = listed.with_file_name(target.file_name().unwrap_or_default());
        app.state::<Library>().moved(listed, &renamed);
        Ok(renamed.display().to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

fn rename(file: &Path, name: &str) -> Result<PathBuf, String> {
    let target = clip::unique_path_except(clip::named_path(file, name)?, Some(file));
    if target != file {
        std::fs::rename(file, &target)
            .map_err(|e| format!("couldn't rename {}: {e}; is it open in another app?", file.display()))?;
    }
    Ok(target)
}

impl Library {
    /// Carries a renamed recording's probe and thumbnail over to its new
    /// path, so neither has to be made again.
    fn moved(&self, old: &Path, new: &Path) {
        if old == new {
            return;
        }
        let mut inner = self.inner.lock().unwrap();
        let Some(entry) = inner.cache.remove(old) else { return };
        self.thumbs.moved(&old.display().to_string(), &new.display().to_string(), entry.size, entry.modified_ms / 1000);
        inner.cache.insert(new.to_owned(), entry);
        save_cache(&paths::library_cache(), &inner.cache);
    }
}

/// Moves a recording to the Recycle Bin.
#[tauri::command]
pub async fn delete_recording(path: String) -> Result<(), String> {
    let file = checked(&path)?;
    tauri::async_runtime::spawn_blocking(move || recycle(&file)).await.map_err(|e| e.to_string())?
}

fn recycle(file: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::UI::Shell::{
        SHFileOperationW, FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_NOERRORUI, FOF_SILENT, FO_DELETE, SHFILEOPSTRUCTW,
    };
    // A list of paths, each NUL-terminated, ending with an extra NUL.
    let from: Vec<u16> = file.as_os_str().encode_wide().chain([0, 0]).collect();
    let mut op = SHFILEOPSTRUCTW {
        wFunc: FO_DELETE,
        pFrom: windows::core::PCWSTR(from.as_ptr()),
        fFlags: (FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_NOERRORUI | FOF_SILENT).0 as u16,
        ..Default::default()
    };
    let code = unsafe { SHFileOperationW(&mut op) };
    if code != 0 || op.fAnyOperationsAborted.as_bool() {
        return Err(format!("couldn't delete {} (error {code:#x}); is it open in another app?", file.display()));
    }
    Ok(())
}

#[tauri::command]
pub async fn show_in_explorer(path: String) -> Result<(), String> {
    ptv_common::process::show_in_explorer(&checked(&path)?);
    Ok(())
}

/// Opens the output folder in Explorer.
#[tauri::command]
pub async fn open_output_folder() -> Result<(), String> {
    let cfg = Config::load_or_create(&paths::config_file()).map_err(|e| format!("{e:#}"))?;
    ptv_common::process::open_folder(&cfg.output_dir);
    Ok(())
}

/// Opens a recording in the default video player.
#[tauri::command]
pub async fn open_external(app: AppHandle, path: String) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let file = checked(&path)?;
    app.opener().open_path(file.display().to_string(), None::<&str>).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempDir(PathBuf);
    impl TempDir {
        fn new(tag: &str) -> Self {
            let p = std::env::temp_dir().join(format!("ptv-library-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&p);
            std::fs::create_dir_all(&p).unwrap();
            TempDir(p)
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn touch(p: &Path) -> PathBuf {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, b"x").unwrap();
        p.to_owned()
    }

    #[test]
    fn only_our_recordings_in_the_output_folder_are_allowed() {
        let t = TempDir::new("guard");
        let out = t.0.join("out");
        let clip = touch(&out.join("RuneLite").join("RuneLite_2026-10-21_14-03-22.mp4"));
        let session = touch(&out.join("RuneLite").join("Sessions").join("RuneLite_session_2026-10-21_14-03-22.mp4"));
        assert_eq!(own_recording(&clip, &out).unwrap(), strip_verbatim(clip.canonicalize().unwrap()));
        assert!(own_recording(&session, &out).is_ok());

        // Right name, but outside the output folder.
        let outside = touch(&t.0.join("elsewhere").join("RuneLite").join("RuneLite_2026-10-21_14-03-22.mp4"));
        assert!(own_recording(&outside, &out).is_err());
        // Escaping with `..` doesn't help.
        let sneaky = out
            .join("RuneLite")
            .join("..")
            .join("..")
            .join("elsewhere")
            .join("RuneLite")
            .join(outside.file_name().unwrap());
        assert!(own_recording(&sneaky, &out).is_err());
        // Inside, but not one of our file names.
        let foreign = touch(&out.join("RuneLite").join("holiday.mp4"));
        assert!(own_recording(&foreign, &out).unwrap_err().contains("isn't a playerstv recording"));
        // Our name, but not in a game folder.
        let loose = touch(&out.join("RuneLite_2026-10-21_14-03-22.mp4"));
        assert!(own_recording(&loose, &out).is_err());
        // Missing files.
        assert!(own_recording(&out.join("RuneLite").join("RuneLite_2000-01-01_00-00-00.mp4"), &out).is_err());
    }

    #[test]
    fn cache_is_reused_until_the_file_changes() {
        let t = TempDir::new("cache");
        let clip = touch(&t.0.join("RuneLite").join("RuneLite_2026-10-21_14-03-22.mp4"));
        let session = touch(&t.0.join("RuneLite").join("Sessions").join("RuneLite_session_2026-10-21_15-00-00.mp4"));
        let probes = std::cell::Cell::new(0);
        let probe = |_: &Path| {
            probes.set(probes.get() + 1);
            Some(MediaInfo { duration_s: 1.0, video: None, audio_tracks: vec![] })
        };
        let mut cache = HashMap::new();

        let (recs, changed) = build(storage::scan(&t.0), &mut cache, probe);
        assert_eq!((recs.len(), probes.get(), changed), (2, 2, true));
        assert!(recs.iter().any(|r| r.kind == Kind::Session && r.game == "RuneLite"));
        assert!(recs.iter().any(|r| r.kind == Kind::Clip && r.game == "RuneLite"));

        // Unchanged files: no probing, nothing to save.
        let (_, changed) = build(storage::scan(&t.0), &mut cache, probe);
        assert_eq!((probes.get(), changed), (2, false));

        // A file that grew is probed again.
        std::fs::write(&clip, b"longer").unwrap();
        let (_, changed) = build(storage::scan(&t.0), &mut cache, probe);
        assert_eq!((probes.get(), changed), (3, true));

        // A deleted file drops out of the cache.
        std::fs::remove_file(&session).unwrap();
        let (recs, changed) = build(storage::scan(&t.0), &mut cache, probe);
        assert_eq!((recs.len(), cache.len(), probes.get(), changed), (1, 1, 3, true));

        // And the cache survives a save/load round trip.
        let file = t.0.join("library.json");
        save_cache(&file, &cache);
        assert_eq!(load_cache(&file), cache);
    }
}
