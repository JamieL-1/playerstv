//! Writing clips and sessions: stream-copy buffer segments into an .mp4.

use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{bail, Context, Result};
use ptv_core::clip::{self, ExportKind};
use ptv_core::ring::Segment;

use crate::recorder::{bind_to_our_lifetime, CREATE_NO_WINDOW};

pub fn export(
    ffmpeg: &Path,
    segments: &[Segment],
    output: PathBuf,
    audio_titles: &[String],
    kind: ExportKind,
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
    let result = Command::new(ffmpeg)
        .args(clip::export_args(&list, &output, &titles, kind))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .and_then(|child| {
            bind_to_our_lifetime(&child);
            child.wait_with_output()
        });
    let _ = std::fs::remove_file(&list);

    let out = result.with_context(|| format!("running {}", ffmpeg.display()))?;
    if !out.status.success() {
        let _ = std::fs::remove_file(&output);
        bail!("ffmpeg export failed: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(output)
}
