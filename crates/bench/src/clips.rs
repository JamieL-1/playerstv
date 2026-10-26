//! Spotting the clip a recorder saves and timing how long the save takes.

use std::collections::HashSet;
use std::os::windows::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{bail, Result};

const EXTENSIONS: [&str; 6] = ["mp4", "mkv", "mov", "ts", "flv", "webm"];
const POLL: Duration = Duration::from_millis(50);
/// A finished clip must stay the same size this long, so a recorder that
/// writes in passes (e.g. moving the index to the front) isn't caught
/// halfway.
const SETTLE: Duration = Duration::from_millis(750);

/// Every video file under `dir`, recursively.
pub fn snapshot(dir: &Path) -> HashSet<PathBuf> {
    let mut out = HashSet::new();
    let mut stack = vec![dir.to_owned()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else { continue };
        for e in entries.flatten() {
            let path = e.path();
            if e.file_type().is_ok_and(|t| t.is_dir()) {
                stack.push(path);
            } else if is_video(&path) {
                out.insert(path);
            }
        }
    }
    out
}

fn is_video(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
}

/// Opening with no sharing fails while any other process has it open.
fn closed_by_writer(path: &Path) -> bool {
    std::fs::OpenOptions::new().read(true).share_mode(0).open(path).is_ok()
}

/// Waits for a video file under `dir` that isn't in `before` to be
/// complete: closed by its writer, non-empty, and not changing. Returns it
/// and when it first looked complete, measured from `since`.
pub fn wait_for_new(
    dir: &Path,
    before: &HashSet<PathBuf>,
    since: Instant,
    timeout: Duration,
) -> Result<(PathBuf, Duration)> {
    let deadline = since + timeout;
    // (path, size, when it first looked complete)
    let mut candidate: Option<(PathBuf, u64, Instant)> = None;
    while Instant::now() < deadline {
        let now = Instant::now();
        let mut fresh: Vec<PathBuf> = snapshot(dir).difference(before).cloned().collect();
        // Newest first: a recorder writing a temp file and renaming it makes
        // the renamed one the newest.
        fresh.sort_by_key(|p| std::cmp::Reverse(std::fs::metadata(p).and_then(|m| m.modified()).ok()));
        if let Some(path) = fresh.into_iter().next() {
            let size = std::fs::metadata(&path).map_or(0, |m| m.len());
            let done = size > 0 && closed_by_writer(&path);
            candidate = match candidate.take() {
                Some((p, s, at)) if done && p == path && s == size => {
                    if now - at >= SETTLE {
                        return Ok((path, at - since));
                    }
                    Some((p, s, at))
                }
                _ if done => Some((path, size, now)),
                _ => None,
            };
        }
        std::thread::sleep(POLL);
    }
    bail!("no new clip appeared in {} within {} s", dir.display(), timeout.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_a_new_finished_clip() {
        let dir = std::env::temp_dir().join(format!("ptv-clips-test-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("Game")).unwrap();
        std::fs::write(dir.join("old.mp4"), b"old").unwrap();
        std::fs::write(dir.join("notes.txt"), b"not a video").unwrap();
        let before = snapshot(&dir);
        assert_eq!(before.len(), 1);

        let since = Instant::now();
        let writer = {
            let path = dir.join("Game").join("new.mp4");
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(200));
                std::fs::write(path, b"clip").unwrap();
            })
        };
        let (path, took) = wait_for_new(&dir, &before, since, Duration::from_secs(5)).unwrap();
        writer.join().unwrap();
        assert_eq!(path, dir.join("Game").join("new.mp4"));
        assert!(took >= Duration::from_millis(200) && took < Duration::from_secs(2), "{took:?}");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
