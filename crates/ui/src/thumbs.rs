//! Library thumbnails: one JPEG per recording, made in the background by
//! ffmpeg and cached under `%LOCALAPPDATA%\playerstv\thumbs`. Each finished
//! thumbnail is announced with the `thumbnail-ready` event.

use std::collections::HashSet;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ptv_common::paths;
use ptv_common::process::{bind_to_our_lifetime, CREATE_NO_WINDOW};
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
        let (tx, rx) = std::sync::mpsc::channel::<Job>();
        let rx = Arc::new(Mutex::new(rx));
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
        let mut keep = HashSet::new();
        let mut tried = self.tried.lock().unwrap();
        for r in recs.iter_mut() {
            // Still being written (or unreadable): nothing to show yet.
            let Some(info) = &r.info else { continue };
            let thumb = self.dir.join(format!("{}.jpg", key(&r.path, r.size, r.modified)));
            keep.insert(thumb.clone());
            if thumb.exists() {
                r.thumb = Some(thumb.display().to_string());
            } else if tried.insert(thumb.clone()) {
                let job = Job { source: r.path.clone(), thumb, at_s: info.duration_s * 0.1 };
                let _ = self.jobs.send(job);
            }
        }
        prune(&self.dir, &keep);
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

fn prune(dir: &Path, keep: &HashSet<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for path in entries.filter_map(|e| e.ok()).map(|e| e.path()) {
        let ours = path.extension().is_some_and(|e| e == "jpg")
            && path
                .file_stem()
                .and_then(|s| s.to_str())
                .is_some_and(|s| s.len() == 16 && s.bytes().all(|b| b.is_ascii_hexdigit()));
        if ours && !keep.contains(&path) {
            let _ = std::fs::remove_file(&path);
        }
    }
}

fn work(rx: &Mutex<Receiver<Job>>, app: &AppHandle) {
    loop {
        let Ok(job) = rx.lock().unwrap().recv() else { return };
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
    let mut child = Command::new(paths::ffmpeg_exe())
        .args(args(&job.source, job.at_s, &part))
        .creation_flags(CREATE_NO_WINDOW)
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
            let _ = std::fs::remove_file(&part);
            return Err("ffmpeg timed out".into());
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    if !status.success() || !part.exists() {
        let _ = std::fs::remove_file(&part);
        return Err(format!("ffmpeg failed ({status})"));
    }
    std::fs::rename(&part, &job.thumb).map_err(|e| e.to_string())
}

fn args(source: &str, at_s: f64, out: &Path) -> Vec<String> {
    let mut a: Vec<String> = ["-hide_banner", "-nostdin", "-loglevel", "error", "-y"].map(String::from).to_vec();
    // Before -i: a fast keyframe seek, which is plenty for a thumbnail.
    a.extend(["-ss".into(), format!("{at_s:.3}"), "-i".into(), source.to_owned()]);
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
        prune(&dir, &HashSet::from([keep.clone()]));
        let left = (keep.exists(), stale.exists(), other.exists());
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(left, (true, false, true));
    }

    #[test]
    fn seeks_before_input() {
        let a = args(r"C:\v\a.mp4", 1.25, Path::new(r"C:\t\k.part")).join(" ");
        assert!(a.contains(r"-ss 1.250 -i C:\v\a.mp4 -frames:v 1 -vf scale=480:-2"), "{a}");
        assert!(a.ends_with(r"-f mjpeg C:\t\k.part"));
    }
}
