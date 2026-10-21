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

/// Starts the tray app (it sits next to this exe) when it isn't running.
#[tauri::command]
pub async fn start_tray() -> Result<(), String> {
    if control::tray_running() {
        return Ok(());
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let tray = exe.with_file_name("playerstv.exe");
    std::process::Command::new(&tray)
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("couldn't start {}: {e}", tray.display()))
}
