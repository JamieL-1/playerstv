//! Library thumbnails: one JPEG per recording, made in the background by
//! ffmpeg and cached under `%LOCALAPPDATA%\playerstv\thumbs`. Each finished
//! thumbnail is announced with the `thumbnail-ready` event.

use std::collections::HashSet;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender};
use ptv_common::paths;
use ptv_common::process::{bind_to_our_lifetime, BELOW_NORMAL_PRIORITY_CLASS, CREATE_NO_WINDOW};
use ptv_common::sync::lock;
use ptv_core::clip::Decode;
use tauri::{AppHandle, Emitter};

use crate::library::Recording;

const WORKERS: usize = 2;
const TIMEOUT: Duration = Duration::from_secs(30);

struct Job {
    source: String,
    thumb: PathBuf,
    at_s: f64,
}

#[derive(Clone, serde::Serialize)]
struct Ready {
    path: String,
    thumb: String,
}

pub struct Thumbs {
    dir: PathBuf,
    jobs: Sender<Job>,
    /// Thumbnails already queued this run, so a failure isn't retried on
    /// every library refresh.
    tried: Mutex<HashSet<PathBuf>>,
}

impl Thumbs {
    pub fn start(app: AppHandle) -> Self {
        let (tx, rx) = crossbeam_channel::unbounded::<Job>();
        for i in 0..WORKERS {
            let (rx, app) = (rx.clone(), app.clone());
            std::thread::Builder::new()
                .name(format!("thumbs-{i}"))
                .spawn(move || work(&rx, &app))
                .expect("spawn thumbnail worker");
        }
        Thumbs { dir: paths::thumbs_dir(), jobs: tx, tried: Mutex::new(HashSet::new()) }
    }

    /// Fills in `thumb` where one is cached, queues the rest (in the given
    /// order, so newest first) and deletes thumbnails of files that are gone.
    pub fn attach(&self, recs: &mut [Recording]) {
        // One listing of the folder answers "is it cached?" for every
        // recording, where asking the disk about each took as many calls.
        let cached = list(&self.dir);
        let mut keep = HashSet::new();
        let mut tried = lock(&self.tried);
        for r in recs.iter_mut() {
            // Still being written (or unreadable): nothing to show yet.
            let Some(info) = &r.info else { continue };
            let thumb = self.dir.join(format!("{}.jpg", key(&r.path, r.size, r.modified)));
            keep.insert(thumb.clone());
            if cached.contains(&thumb) {
                r.thumb = Some(thumb.display().to_string());
            } else if tried.insert(thumb.clone()) {
                let job = Job { source: r.path.clone(), thumb, at_s: info.duration_s * 0.1 };
                let _ = self.jobs.send(job);
            }
        }
        prune(&cached, &keep);
    }

    /// Keeps a recording's thumbnail when it's renamed from `old` to `new`.
    pub fn moved(&self, old: &str, new: &str, size: u64, modified: u64) {
        let from = self.dir.join(format!("{}.jpg", key(old, size, modified)));
        let _ = std::fs::rename(from, self.dir.join(format!("{}.jpg", key(new, size, modified))));
    }
}

/// Stable across runs (unlike `DefaultHasher`): FNV-1a of path, size, mtime.
fn key(path: &str, size: u64, modified: u64) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in path.bytes().chain(size.to_le_bytes()).chain(modified.to_le_bytes()) {
        h ^= b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{h:016x}")
}

/// Every file in the thumbnail folder.
fn list(dir: &Path) -> HashSet<PathBuf> {
    std::fs::read_dir(dir).map(|d| d.filter_map(|e| e.ok()).map(|e| e.path()).collect()).unwrap_or_default()
}

/// Deletes the thumbnails among `files` that aren't in `keep`.
fn prune(files: &HashSet<PathBuf>, keep: &HashSet<PathBuf>) {
    for path in files {
        let ours = path.extension().is_some_and(|e| e == "jpg")
            && path
                .file_stem()
                .and_then(|s| s.to_str())
                .is_some_and(|s| s.len() == 16 && s.bytes().all(|b| b.is_ascii_hexdigit()));
        if ours && !keep.contains(path) {
            let _ = std::fs::remove_file(path);
        }
    }
}

fn work(rx: &Receiver<Job>, app: &AppHandle) {
    for job in rx {
        match make(&job) {
            Ok(()) => {
                let ready = Ready { path: job.source, thumb: job.thumb.display().to_string() };
                let _ = app.emit("thumbnail-ready", ready);
            }
            Err(e) => tracing::warn!("thumbnail for {}: {e}", job.source),
        }
    }
}

fn make(job: &Job) -> Result<(), String> {
    if let Some(dir) = job.thumb.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    // Written under another name and renamed, so the window never loads a
    // half-written JPEG.
    let part = job.thumb.with_extension("part");
    // While a game is being recorded, thumbnails keep out of its way: one
    // at a time, at low priority, and off the GPU where the CPU can do it.
    let recording = crate::status::recording();
    let _turn = recording.then(|| lock(&ONE_AT_A_TIME));
    let [first, second] = decode_order(recording, GPU_FAILURES.load(Ordering::Relaxed));
    // An explicit `-hwaccel cuda` fails outright, rather than decoding in
    // software, when CUDA can't start (no NVIDIA driver, say), and the CPU
    // can't decode everything (AV1), so whichever is tried first, a failure
    // is retried on the other.
    let used = match extract(job, &part, first, recording) {
        Ok(()) => first,
        Err(e) => {
            tracing::debug!("thumbnail for {} with {first:?} decoding: {e}; trying {second:?}", job.source);
            extract(job, &part, second, recording)?;
            second
        }
    };
    match (first, used) {
        (Decode::Gpu, Decode::Gpu) => GPU_FAILURES.store(0, Ordering::Relaxed),
        // The file was fine, so the GPU was what failed.
        (Decode::Gpu, Decode::Cpu) => {
            GPU_FAILURES.fetch_add(1, Ordering::Relaxed);
        }
        _ => {}
    }
    std::fs::rename(&part, &job.thumb).map_err(|e| e.to_string())
}

/// Held while making a thumbnail during a recording, so the workers take
/// turns instead of each starting an ffmpeg beside the game.
static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

/// GPU decodes in a row that failed on files the CPU then decoded.
static GPU_FAILURES: AtomicU32 = AtomicU32::new(0);
/// After this many, the GPU is taken to be unusable here (no NVIDIA
/// driver, say) and is only tried when the CPU fails, saving a doomed
/// ffmpeg launch per thumbnail.
const GPU_GIVEN_UP_AFTER: u32 = 3;

/// Which decoder to try first, and which if that fails. The GPU is faster,
/// but starting a decode on it costs the game video memory and a hitch, so
/// not while one is being recorded.
fn decode_order(recording: bool, gpu_failures: u32) -> [Decode; 2] {
    if recording || gpu_failures >= GPU_GIVEN_UP_AFTER {
        [Decode::Cpu, Decode::Gpu]
    } else {
        [Decode::Gpu, Decode::Cpu]
    }
}

/// Runs ffmpeg to write the thumbnail to `part`.
fn extract(job: &Job, part: &Path, decode: Decode, low_priority: bool) -> Result<(), String> {
    let priority = if low_priority { BELOW_NORMAL_PRIORITY_CLASS } else { 0 };
    let mut child = Command::new(paths::ffmpeg_exe())
        .args(args(&job.source, job.at_s, part, decode))
        .creation_flags(CREATE_NO_WINDOW | priority)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("couldn't start ffmpeg: {e}"))?;
    bind_to_our_lifetime(&child);
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            break status;
        }
        if started.elapsed() > TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            let _ = std::fs::remove_file(part);
            return Err("ffmpeg timed out".into());
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    if !status.success() || !part.exists() {
        let _ = std::fs::remove_file(part);
        return Err(format!("ffmpeg failed ({status})"));
    }
    Ok(())
}

fn args(source: &str, at_s: f64, out: &Path, decode: Decode) -> Vec<String> {
    let mut a: Vec<String> = ["-hide_banner", "-nostdin", "-loglevel", "error", "-y"].map(String::from).to_vec();
    // NVDEC when it can (FFmpeg's software AV1 decoder can't read 4K120 AV1
    // at all); frames come back to the CPU for scaling.
    if decode == Decode::Gpu {
        a.extend(["-hwaccel", "cuda"].map(String::from));
    }
    // Any frame near `at_s` makes a fine thumbnail, so only keyframes are
    // decoded and the one at or before `at_s` is used as it is: a fraction
    // of the work of decoding up to it.
    a.extend(["-skip_frame", "nokey", "-noaccurate_seek", "-ss"].map(String::from));
    a.extend([format!("{at_s:.3}"), "-i".into(), source.to_owned()]);
    a.extend(["-frames:v", "1", "-vf", "scale=480:-2", "-q:v", "4", "-f", "mjpeg"].map(String::from));
    a.push(out.display().to_string());
    a
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_change_with_the_file() {
        let k = key(r"C:\v\a.mp4", 10, 100);
        assert_eq!(k.len(), 16);
        assert_eq!(k, key(r"C:\v\a.mp4", 10, 100));
        assert_ne!(k, key(r"C:\v\a.mp4", 11, 100));
        assert_ne!(k, key(r"C:\v\a.mp4", 10, 101));
        assert_ne!(k, key(r"C:\v\b.mp4", 10, 100));
    }

    #[test]
    fn prune_removes_only_stale_thumbnails() {
        let dir = std::env::temp_dir().join(format!("ptv-thumbs-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let keep = dir.join("0123456789abcdef.jpg");
        let stale = dir.join("fedcba9876543210.jpg");
        let other = dir.join("notes.jpg");
        for p in [&keep, &stale, &other] {
            std::fs::write(p, b"x").unwrap();
        }
        let files = list(&dir);
        assert_eq!(files.len(), 3);
        prune(&files, &HashSet::from([keep.clone()]));
        let left = (keep.exists(), stale.exists(), other.exists());
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(left, (true, false, true));
    }

    #[test]
    fn the_gpu_decodes_first_unless_a_game_is_recording_or_it_keeps_failing() {
        assert_eq!(decode_order(false, 0), [Decode::Gpu, Decode::Cpu]);
        assert_eq!(decode_order(false, GPU_GIVEN_UP_AFTER - 1), [Decode::Gpu, Decode::Cpu]);
        assert_eq!(decode_order(false, GPU_GIVEN_UP_AFTER), [Decode::Cpu, Decode::Gpu]);
        assert_eq!(decode_order(true, 0), [Decode::Cpu, Decode::Gpu]);
    }

    #[test]
    fn seeks_before_input() {
        let a = args(r"C:\v\a.mp4", 1.25, Path::new(r"C:\t\k.part"), Decode::Gpu).join(" ");
        assert!(a.contains("-hwaccel cuda -skip_frame nokey -noaccurate_seek"), "{a}");
        assert!(a.contains(r"-noaccurate_seek -ss 1.250 -i C:\v\a.mp4 -frames:v 1 -vf scale=480:-2"), "{a}");
        assert!(a.ends_with(r"-f mjpeg C:\t\k.part"));
    }

    #[test]
    fn the_cpu_fallback_asks_for_no_hardware() {
        let a = args(r"C:\v\a.mp4", 1.25, Path::new(r"C:\t\k.part"), Decode::Cpu).join(" ");
        assert!(!a.contains("hwaccel"), "{a}");
        assert!(a.contains("-skip_frame nokey -noaccurate_seek -ss 1.250"), "{a}");
    }
}
