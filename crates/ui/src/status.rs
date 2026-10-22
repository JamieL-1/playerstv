//! Live recording status from the tray app, pushed to the window as the
//! `status` event (a `Snapshot`, or `null` while the tray isn't running).

use std::os::windows::process::CommandExt;
use std::time::Duration;

use ptv_common::control::{self, Request, Snapshot};
use ptv_common::process::CREATE_NO_WINDOW;
use tauri::{AppHandle, Emitter};

const POLL: Duration = Duration::from_millis(500);

/// Polls the tray for the life of the app and emits every result.
pub fn start_poller(app: AppHandle) {
    std::thread::Builder::new()
        .name("status-poller".into())
        .spawn(move || loop {
            let snapshot = control::request(Request::Status).ok();
            let _ = app.emit("status", &snapshot);
            std::thread::sleep(POLL);
        })
        .expect("spawn status poller");
}

/// Closes the window when the tray quits (from its menu or [`quit_app`]).
pub fn close_with_tray(app: AppHandle) {
    let signal = match control::QuitSignal::open() {
        Ok(s) => s,
        Err(e) => return eprintln!("playerstv: no quit event: {e:#}"),
    };
    std::thread::Builder::new()
        .name("quit-watcher".into())
        .spawn(move || {
            signal.wait();
            app.exit(0);
        })
        .expect("spawn quit watcher");
}

/// Quits playerstv: the tray stops recording and exits, and the window
/// closes with it (straight away if the tray isn't running).
#[tauri::command]
pub async fn quit_app(app: AppHandle) {
    if control::request(Request::Quit).is_err() {
        app.exit(0);
    }
}

fn send(req: Request) -> Result<Snapshot, String> {
    control::request(req).map_err(|e| format!("{e:#}"))
}

#[tauri::command]
pub async fn get_status() -> Option<Snapshot> {
    control::request(Request::Status).ok()
}

#[tauri::command]
pub async fn save_clip() -> Result<Snapshot, String> {
    send(Request::SaveClip)
}

#[tauri::command]
pub async fn toggle_session() -> Result<Snapshot, String> {
    send(Request::ToggleSession)
}

/// Installs the update the tray downloaded; playerstv (window included)
/// closes and starts again on the new version.
#[tauri::command]
pub async fn restart_to_update() -> Result<Snapshot, String> {
    send(Request::Restart)
}

/// Starts the tray (this same exe, without opening another window) when it
/// isn't running, e.g. after Quit from the tray menu.
#[tauri::command]
pub async fn start_tray() -> Result<(), String> {
    if control::tray_running() {
        return Ok(());
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    std::process::Command::new(&exe)
        .arg(playerstv_tray::BACKGROUND_FLAG)
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("couldn't start {}: {e}", exe.display()))
}
