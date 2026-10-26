//! Exporting a cut of a recording as a new clip (see
//! `ptv_core::clip::trim_args`). Reports `export-progress {id, fraction}`
//! while ffmpeg runs, then one `export-done` event.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStderr, ChildStdout, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use ptv_common::paths;
use ptv_common::process::{bind_to_our_lifetime, CREATE_NO_WINDOW};
use ptv_common::sync::lock;
use ptv_core::clip::{self, Decode, ExportKind, TrimSpec};
use ptv_core::config::{Codec, Config};
use tauri::{AppHandle, Emitter, Manager};

use crate::library;

/// Shorter cuts than this aren't worth a file (and NVENC needs a few frames).
const MIN_LENGTH_S: f64 = 0.1;

#[derive(Default)]
pub struct Exports {
    next_id: AtomicU64,
    running: Mutex<HashMap<u64, Running>>,
}

struct Running {
    child: Arc<Mutex<Child>>,
    cancelled: Arc<AtomicBool>,
}

#[derive(Clone, serde::Serialize)]
struct Progress {
    id: u64,
    fraction: f64,
}

#[derive(Clone, serde::Serialize)]
#[serde(tag = "state", rename_all = "lowercase")]
enum Done {
    Saved { id: u64, path: String },
    Failed { id: u64, error: String },
    Cancelled { id: u64 },
}

/// Starts exporting `start..end` seconds of `path`, saved as `name` (see
/// `clip::named_path`) or the default name; returns the export id.
#[tauri::command]
pub async fn export_trim(
    app: AppHandle,
    path: String,
    start: f64,
    end: f64,
    name: Option<String>,
) -> Result<u64, String> {
    tauri::async_runtime::spawn_blocking(move || start_export(&app, &path, start, end, name.as_deref()))
        .await
        .map_err(|e| e.to_string())?
}

fn start_export(app: &AppHandle, path: &str, start: f64, end: f64, name: Option<&str>) -> Result<u64, String> {
    let cfg = Config::load_or_create(&paths::config_file()).map_err(|e| format!("{e:#}"))?;
    let source = library::own_recording(Path::new(path), &cfg.output_dir)?;
    let info = ptv_core::media::probe(&source).map_err(|e| format!("couldn't read {}: {e:#}", source.display()))?;
    let video = info.video.as_ref().ok_or("this recording has no video")?;
    let (start, end) = (start.max(0.0), end.min(info.duration_s));
    if end - start < MIN_LENGTH_S {
        return Err("the selection is too short".into());
    }
    // Keep the source's codec, so a trim looks like the recording it came from.
    let codec = match video.codec.as_str() {
        "h264" => Codec::H264,
        "hevc" => Codec::Hevc,
        "av1" => Codec::Av1,
        _ => cfg.video.codec,
    };
    let (game, _) = library::game_and_kind(&source);
    let mut output = clip::output_path(&cfg.output_dir, &game, ExportKind::Clip, chrono::Local::now());
    if let Some(name) = name {
        output = clip::named_path(&output, name)?;
    }
    let exports = app.state::<Exports>();
    let id = exports.next_id.fetch_add(1, Ordering::Relaxed);
    let part = output.with_extension(format!("mp4.{id}.part"));
    let titles: Vec<&str> = info.audio_tracks.iter().map(String::as_str).collect();
    // Our own 4:2:0 recordings decode on the GPU, with the CPU as a fallback
    // should NVDEC turn out not to handle them; 4:4:4 ones go straight to it.
    let decode = if video.playable { Decode::Gpu } else { Decode::Cpu };
    let spec = TrimSpec {
        source: &source,
        start,
        end,
        output: &part,
        codec,
        quality: cfg.video.quality,
        audio_titles: &titles,
        decode,
    };
    let args = clip::trim_args(&spec);
    let fallback = (decode == Decode::Gpu).then(|| clip::trim_args(&TrimSpec { decode: Decode::Cpu, ..spec }));

    let (child, stdout, stderr) = spawn_ffmpeg(&args)?;
    let child = Arc::new(Mutex::new(child));
    let cancelled = Arc::new(AtomicBool::new(false));
    lock(&exports.running).insert(id, Running { child: child.clone(), cancelled: cancelled.clone() });

    let app = app.clone();
    std::thread::Builder::new()
        .name(format!("export-{id}"))
        .spawn(move || {
            let length = end - start;
            let mut run = follow(&app, id, length, &child, stdout, stderr);
            let retry_on_cpu = fallback.filter(|_| !run.succeeded() && !cancelled.load(Ordering::SeqCst));
            if let Some(args) = retry_on_cpu {
                tracing::warn!("GPU-decoded trim failed ({}), retrying on the CPU", run.errors.trim());
                run = match spawn_ffmpeg(&args) {
                    Ok((retry, stdout, stderr)) => {
                        *lock(&child) = retry;
                        // Cancelled while the retry was starting: stop it.
                        if cancelled.load(Ordering::SeqCst) {
                            let _ = lock(&child).kill();
                        }
                        follow(&app, id, length, &child, stdout, stderr)
                    }
                    Err(errors) => Run { status: None, errors },
                };
            }
            lock(&app.state::<Exports>().running).remove(&id);
            let done = finish(id, &part, &output, run.status, &run.errors, cancelled.load(Ordering::SeqCst));
            let _ = app.emit("export-done", done);
        })
        .expect("spawn export thread");
    Ok(id)
}

fn spawn_ffmpeg(args: &[String]) -> Result<(Child, ChildStdout, ChildStderr), String> {
    let mut child = Command::new(paths::ffmpeg_exe())
        .args(args)
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("couldn't start ffmpeg: {e}"))?;
    bind_to_our_lifetime(&child);
    let stdout = child.stdout.take().expect("stdout is piped");
    let stderr = child.stderr.take().expect("stderr is piped");
    Ok((child, stdout, stderr))
}

/// How one ffmpeg run ended.
struct Run {
    status: Option<ExitStatus>,
    /// What it printed to stderr.
    errors: String,
}

impl Run {
    fn succeeded(&self) -> bool {
        self.status.is_some_and(|s| s.success())
    }
}

/// Follows one ffmpeg run to its end, emitting its progress.
fn follow(
    app: &AppHandle,
    id: u64,
    length_s: f64,
    child: &Mutex<Child>,
    stdout: ChildStdout,
    stderr: ChildStderr,
) -> Run {
    let errors = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = BufReader::new(stderr).read_to_string(&mut s);
        s
    });
    for line in BufReader::new(stdout).lines().map_while(Result::ok) {
        if let Some(fraction) = parse_progress(&line, length_s) {
            let _ = app.emit("export-progress", Progress { id, fraction });
        }
    }
    // stdout has closed, so ffmpeg is exiting: this lock isn't held for long.
    let status = lock(child).wait().ok();
    Run { status, errors: errors.join().unwrap_or_default() }
}

/// `out_time_us=5000000` → 0.5 of a 10 s cut.
fn parse_progress(line: &str, length_s: f64) -> Option<f64> {
    let us: f64 = line.strip_prefix("out_time_us=")?.trim().parse().ok()?;
    Some((us / 1e6 / length_s).clamp(0.0, 1.0))
}

/// Renames the finished file into place, or removes what's left of it.
fn finish(
    id: u64,
    part: &Path,
    output: &Path,
    status: Option<std::process::ExitStatus>,
    errors: &str,
    cancelled: bool,
) -> Done {
    let ok = status.is_some_and(|s| s.success()) && part.exists();
    if cancelled || !ok {
        let _ = std::fs::remove_file(part);
        if cancelled {
            return Done::Cancelled { id };
        }
        let detail = errors.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("ffmpeg failed").trim();
        return Done::Failed { id, error: format!("Export failed: {detail}") };
    }
    // Another clip may have taken the name while this one was encoding.
    let target: PathBuf = clip::unique_path(output.to_owned());
    match std::fs::rename(part, &target) {
        Ok(()) => Done::Saved { id, path: target.display().to_string() },
        Err(e) => {
            let _ = std::fs::remove_file(part);
            Done::Failed { id, error: format!("couldn't save {}: {e}", target.display()) }
        }
    }
}

#[tauri::command]
pub async fn cancel_trim(app: AppHandle, id: u64) {
    let exports = app.state::<Exports>();
    let running = lock(&exports.running);
    if let Some(r) = running.get(&id) {
        r.cancelled.store(true, Ordering::SeqCst);
        let _ = lock(&r.child).kill();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_lines() {
        assert_eq!(parse_progress("out_time_us=2500000", 5.0), Some(0.5));
        assert_eq!(parse_progress("out_time_us=9000000", 5.0), Some(1.0));
        assert_eq!(parse_progress("out_time_us=N/A", 5.0), None);
        assert_eq!(parse_progress("frame=10", 5.0), None);
    }

    #[test]
    fn failed_or_cancelled_exports_leave_nothing_behind() {
        let dir = std::env::temp_dir().join(format!("ptv-trim-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join("RuneLite_2026-10-22_10-00-00.mp4");
        let part = dir.join("RuneLite_2026-10-22_10-00-00.mp4.0.part");

        std::fs::write(&part, b"half").unwrap();
        assert!(matches!(finish(0, &part, &out, None, "", true), Done::Cancelled { .. }));
        assert!(!part.exists() && !out.exists());

        std::fs::write(&part, b"half").unwrap();
        let Done::Failed { error, .. } = finish(0, &part, &out, None, "x\nNo NVENC capable devices found\n", false)
        else {
            panic!("expected a failure");
        };
        assert!(error.contains("No NVENC capable devices found"));
        assert!(!part.exists() && !out.exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
