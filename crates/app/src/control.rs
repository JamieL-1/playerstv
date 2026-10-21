//! Local control channel between the tray process and the playerstv window
//! (`--settings`): a named pipe speaking one line of text each way.
//!
//! Requests: `status`, `clip`, `session`. Every reply is a JSON [`Snapshot`].

use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::os::windows::io::FromRawHandle;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use windows::Win32::Foundation::{ERROR_PIPE_CONNECTED, HANDLE};
use windows::Win32::Storage::FileSystem::{FlushFileBuffers, PIPE_ACCESS_DUPLEX};
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_READMODE_BYTE,
    PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
};

use crate::supervisor::Status;

const ERROR_PIPE_BUSY: i32 = 231;

/// Everything the window needs to draw its status card.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub status: Status,
    pub buffer_seconds: u32,
    pub clip_hotkey: String,
    pub session_hotkey: String,
    pub output_dir: PathBuf,
    pub last_saved: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Request {
    Status,
    SaveClip,
    ToggleSession,
}

impl Request {
    fn as_str(self) -> &'static str {
        match self {
            Request::Status => "status",
            Request::SaveClip => "clip",
            Request::ToggleSession => "session",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        [Request::Status, Request::SaveClip, Request::ToggleSession]
            .into_iter()
            .find(|r| r.as_str() == s)
    }
}

/// Runs the server on a background thread for the life of the process.
pub fn serve(snapshot: Arc<Mutex<Snapshot>>, on_request: impl Fn(Request) + Send + 'static) {
    let spawned = std::thread::Builder::new().name("control".into()).spawn(move || loop {
        if let Err(e) = serve_one(&snapshot, &on_request) {
            tracing::debug!("control connection: {e:#}");
            std::thread::sleep(Duration::from_millis(200));
        }
    });
    if let Err(e) = spawned {
        tracing::error!("couldn't start control server: {e}");
    }
}

fn serve_one(snapshot: &Mutex<Snapshot>, on_request: &impl Fn(Request)) -> Result<()> {
    let handle = unsafe {
        CreateNamedPipeW(
            &windows::core::HSTRING::from(crate::paths::control_pipe()),
            PIPE_ACCESS_DUPLEX,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            PIPE_UNLIMITED_INSTANCES,
            4096,
            4096,
            0,
            None,
        )
    };
    if handle.is_invalid() {
        bail!("CreateNamedPipe failed: {}", windows::core::Error::from_thread());
    }
    // From here the File owns (and will close) the handle.
    let file = unsafe { File::from_raw_handle(handle.0) };
    match unsafe { ConnectNamedPipe(handle, None) } {
        Ok(()) => {}
        Err(e) if e.code() == ERROR_PIPE_CONNECTED.to_hresult() => {}
        Err(e) => return Err(e).context("ConnectNamedPipe"),
    }
    let result = answer(&file, snapshot, on_request);
    unsafe {
        let _ = FlushFileBuffers(HANDLE(handle.0));
        let _ = DisconnectNamedPipe(HANDLE(handle.0));
    }
    result
}

fn answer(file: &File, snapshot: &Mutex<Snapshot>, on_request: &impl Fn(Request)) -> Result<()> {
    let mut line = String::new();
    BufReader::new(file).read_line(&mut line)?;
    let request = Request::parse(line.trim()).with_context(|| format!("unknown request {line:?}"))?;
    if request != Request::Status {
        on_request(request);
    }
    let reply = serde_json::to_string(&*snapshot.lock().unwrap())?;
    let mut file = file;
    writeln!(file, "{reply}")?;
    Ok(())
}

/// Sends one request to the running tray app. Errors if it isn't running.
pub fn request(req: Request) -> Result<Snapshot> {
    let mut attempts = 0;
    let mut pipe = loop {
        match OpenOptions::new().read(true).write(true).open(crate::paths::control_pipe()) {
            Ok(f) => break f,
            // All instances busy for a moment: retry briefly.
            Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY) && attempts < 20 => {
                attempts += 1;
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(e) => return Err(e).context("playerstv isn't running"),
        }
    };
    writeln!(pipe, "{}", req.as_str())?;
    let mut line = String::new();
    BufReader::new(pipe).read_line(&mut line)?;
    serde_json::from_str(&line).context("bad reply from playerstv")
}
