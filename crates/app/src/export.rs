//! Writing clips and sessions: stream-copy buffer segments into an .mp4.

use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use ptv_common::process::{lower_io_priority, BELOW_NORMAL_PRIORITY_CLASS};
use ptv_core::clip::{self, ExportKind};
use ptv_core::ring::Segment;

use crate::recorder::{bind_to_our_lifetime, CREATE_NO_WINDOW};

/// How often a running export's output is checked for growth.
const PROGRESS_POLL: Duration = Duration::from_millis(250);

/// How an export runs, beyond what it writes.
pub struct Run<'a> {
    /// Give way to everything else: below-normal CPU and low disk priority.
    /// For a big copy made while a game is still running, which would
    /// otherwise compete with the game and with the recording's own writes.
    pub background: bool,
    /// Called each time the output file is seen to have grown, so whoever
    /// waits for the export can tell a slow one from a stuck one.
    pub on_progress: &'a (dyn Fn() + Sync),
}

pub fn export(
    ffmpeg: &Path,
    segments: &[Segment],
    output: PathBuf,
    audio_titles: &[String],
    kind: ExportKind,
    run: &Run,
) -> Result<PathBuf> {
    if segments.is_empty() {
        bail!("nothing recorded yet");
    }
    let output = clip::unique_path(output);
    if let Some(dir) = output.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    let list = output.with_extension("concat.txt");
    std::fs::write(&list, clip::concat_list(segments))?;

    let titles: Vec<&str> = audio_titles.iter().map(String::as_str).collect();
    let priority = if run.background { BELOW_NORMAL_PRIORITY_CLASS } else { 0 };
    let result = Command::new(ffmpeg)
        .args(clip::export_args(&list, &output, &titles, kind))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .creation_flags(CREATE_NO_WINDOW | priority)
        .spawn()
        .and_then(|child| {
            bind_to_our_lifetime(&child);
            if run.background {
                lower_io_priority(&child);
            }
            // Watched from a second thread, so this one returns the moment
            // ffmpeg exits rather than at the next poll.
            let done = AtomicBool::new(false);
            std::thread::scope(|scope| {
                let watcher = scope.spawn(|| {
                    let mut size = 0;
                    while !done.load(Ordering::Relaxed) {
                        std::thread::park_timeout(PROGRESS_POLL);
                        let now = std::fs::metadata(&output).map_or(size, |m| m.len());
                        if now != size {
                            size = now;
                            (run.on_progress)();
                        }
                    }
                });
                let out = child.wait_with_output();
                done.store(true, Ordering::Relaxed);
                watcher.thread().unpark();
                out
            })
        });
    let _ = std::fs::remove_file(&list);

    let out = result.with_context(|| format!("running {}", ffmpeg.display()))?;
    if !out.status.success() {
        let _ = std::fs::remove_file(&output);
        bail!("ffmpeg export failed: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(output)
}
