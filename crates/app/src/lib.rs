//! The tray side of `playerstv.exe`: game detection, recording, hotkeys and
//! the tray icon. The same exe runs the window with `--window` (crates/ui).

mod autostart;
// Shared with the window; re-exported so `crate::paths` etc. keep working.
pub use ptv_common::{control, paths};
mod export;
mod recorder;
mod supervisor;
mod tray;
mod updater;

pub use autostart::BACKGROUND_FLAG;
pub use tray::toast;
/// Runs the app window instead of the tray (handled by the exe's `main`).
pub const WINDOW_FLAG: &str = "--window";
/// `--wait-pid N <args>`: wait for process N (the tray we're replacing after
/// an update) to exit, then run as `<args>`.
const WAIT_PID_FLAG: &str = "--wait-pid";

use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, Result};
use ptv_core::config::Config;
use tracing_subscriber::EnvFilter;
use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
use windows::Win32::System::Threading::CreateMutexW;

/// Everything except `--window`: `--smoke N`, `--background` (tray only, as
/// used by "Start with Windows"), or nothing (tray plus the window).
pub fn main(mut args: &[String]) {
    if args.first().map(String::as_str) == Some(WAIT_PID_FLAG) {
        if let Some(pid) = args.get(1).and_then(|s| s.parse().ok()) {
            wait_for_exit(pid);
        }
        args = args.get(2..).unwrap_or_default();
    }
    let result = match args.first().map(String::as_str) {
        Some("--smoke") => {
            let secs = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(15);
            smoke(secs)
        }
        Some(autostart::BACKGROUND_FLAG) => run_tray(false),
        _ => run_tray(true),
    };
    if let Err(e) = result {
        tracing::error!("{e:#}");
        eprintln!("playerstv: {e:#}");
        std::process::exit(1);
    }
}

fn run_tray(show_window: bool) -> Result<()> {
    let _guard = init_logging(false);
    // Named mutex: only one tray instance at a time. The handle stays open
    // (and the mutex held) until the process exits.
    unsafe {
        let _mutex = CreateMutexW(None, false, &paths::mutex_name("tray"))?;
        if GetLastError() == ERROR_ALREADY_EXISTS {
            // Launching the app again means "show me the app".
            tracing::info!("already running");
            if show_window {
                tray::open_window();
            }
            return Ok(());
        }
    }
    let cfg = Config::load_or_create(&paths::config_file())?;
    tracing::info!("playerstv {} starting, config {}", env!("CARGO_PKG_VERSION"), paths::config_file().display());
    tracing::info!("using ffmpeg at {}", paths::ffmpeg_exe().display());
    if updater::finish_previous() && cfg.notifications {
        toast(
            &format!("playerstv updated to v{}", env!("CARGO_PKG_VERSION")),
            "See what's new on the GitHub releases page.",
        );
    }
    tray::run(cfg, show_window)
}

/// Starts this exe again as `args` once the current process has exited (and
/// released the single-instance mutex).
fn relaunch_after_exit(args: &[&str]) {
    let exe = match std::env::current_exe() {
        Ok(exe) => exe,
        Err(e) => return tracing::error!("couldn't find own exe: {e}"),
    };
    let pid = std::process::id().to_string();
    if let Err(e) = std::process::Command::new(&exe).arg(WAIT_PID_FLAG).arg(&pid).args(args).spawn() {
        tracing::error!("couldn't restart {}: {e}", exe.display());
    }
}

fn wait_for_exit(pid: u32) {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE};
    // Already gone if it can't be opened.
    if let Ok(process) = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, false, pid) } {
        unsafe {
            // Saving a long session on shutdown can take a while.
            WaitForSingleObject(process, 120_000);
            let _ = CloseHandle(process);
        }
    }
}

/// `playerstv --smoke N`: headless end-to-end check. Waits for the game,
/// records for N seconds, saves a clip and exits.
fn smoke(secs: u64) -> Result<()> {
    attach_console();
    let _guard = init_logging(true);
    let cfg = Config::load_or_create(&paths::config_file())?;
    let ffmpeg = paths::ffmpeg_exe();
    tracing::info!("using ffmpeg at {}", ffmpeg.display());

    let (cmd_tx, cmd_rx) = crossbeam_channel::unbounded();
    let (ev_tx, ev_rx) = crossbeam_channel::unbounded();
    let sink: supervisor::EventSink = Arc::new(move |e| {
        let _ = ev_tx.send(e);
    });
    let sup = supervisor::Supervisor::new(cfg, ffmpeg, sink);
    let handle = std::thread::spawn(move || sup.run(cmd_rx));

    let wait_until = |pred: &dyn Fn(&supervisor::Event) -> bool, timeout: Duration| -> Result<supervisor::Event> {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            match ev_rx.recv_timeout(left) {
                Ok(e) => {
                    tracing::info!("event: {e:?}");
                    if pred(&e) {
                        return Ok(e);
                    }
                }
                Err(_) => bail!("timed out"),
            }
        }
    };

    wait_until(
        &|e| matches!(e, supervisor::Event::Status(supervisor::Status::Recording { .. })),
        Duration::from_secs(30),
    )?;
    tracing::info!("recording for {secs}s...");
    std::thread::sleep(Duration::from_secs(secs));
    cmd_tx.send(supervisor::Command::SaveClip)?;
    let saved = wait_until(
        &|e| matches!(e, supervisor::Event::Saved { .. } | supervisor::Event::Error(_)),
        Duration::from_secs(30),
    )?;
    cmd_tx.send(supervisor::Command::Shutdown)?;
    let _ = handle.join();
    match saved {
        supervisor::Event::Saved { path, .. } => {
            println!("{}", path.display());
            Ok(())
        }
        other => bail!("clip failed: {other:?}"),
    }
}

const LOG_DAYS_KEPT: usize = 14;

fn init_logging(to_stderr: bool) -> Option<tracing_appender::non_blocking::WorkerGuard> {
    let filter = EnvFilter::try_from_env("PLAYERSTV_LOG").unwrap_or_else(|_| EnvFilter::new("info"));
    if to_stderr {
        tracing_subscriber::fmt().with_env_filter(filter).with_writer(std::io::stderr).init();
        return None;
    }
    // Daily files, same names as before (playerstv.log.YYYY-MM-DD), but only
    // the newest two weeks are kept instead of growing forever.
    let appender = tracing_appender::rolling::Builder::new()
        .rotation(tracing_appender::rolling::Rotation::DAILY)
        .filename_prefix("playerstv.log")
        .max_log_files(LOG_DAYS_KEPT)
        .build(paths::log_dir());
    let appender = match appender {
        Ok(a) => a,
        Err(e) => {
            // No log file is better than no app.
            eprintln!("playerstv: logging disabled: {e}");
            return None;
        }
    };
    let (writer, guard) = tracing_appender::non_blocking(appender);
    tracing_subscriber::fmt().with_env_filter(filter).with_ansi(false).with_writer(writer).init();
    Some(guard)
}

/// Release builds have no console; reuse the parent's so CLI modes can print.
fn attach_console() {
    #[cfg(not(debug_assertions))]
    unsafe {
        use windows::Win32::System::Console::{AttachConsole, ATTACH_PARENT_PROCESS};
        let _ = AttachConsole(ATTACH_PARENT_PROCESS);
    }
}
