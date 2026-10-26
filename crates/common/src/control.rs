//! Local control channel between the tray process and the playerstv window
//! (`playerstv.exe --window`): a named pipe speaking one line of text each way.
//!
//! Requests: `status`, `clip`, `session`, `quit`, `restart`, `check-update`.
//! Every reply is a JSON [`Snapshot`].

use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::os::windows::io::{AsRawHandle, FromRawHandle};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use windows::Win32::Foundation::{ERROR_PIPE_CONNECTED, HANDLE};
use windows::Win32::Storage::FileSystem::{FlushFileBuffers, PIPE_ACCESS_DUPLEX};
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS,
    PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
};

use crate::status::Status;

const ERROR_PIPE_BUSY: i32 = 231;
const ERROR_FILE_NOT_FOUND: i32 = 2;

fn is_transient(e: &std::io::Error) -> bool {
    match e.raw_os_error() {
        Some(ERROR_PIPE_BUSY) => true,
        // "No such pipe" is only a gap if the tray is actually running;
        // otherwise report it straight away.
        Some(ERROR_FILE_NOT_FOUND) => tray_running(),
        _ => false,
    }
}

/// Whether the tray app holds its single-instance mutex.
pub fn tray_running() -> bool {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{OpenMutexW, SYNCHRONIZATION_SYNCHRONIZE};
    // SAFETY: plain FFI call; a handle it returns is closed straight away.
    match unsafe { OpenMutexW(SYNCHRONIZATION_SYNCHRONIZE, false, &crate::paths::mutex_name("tray")) } {
        Ok(h) => {
            // SAFETY: `h` was just opened and is closed only here.
            unsafe {
                let _ = CloseHandle(h);
            }
            true
        }
        Err(_) => false,
    }
}

/// Everything the window needs to draw its status card.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    pub status: Status,
    pub buffer_seconds: u32,
    pub clip_hotkey: String,
    pub session_hotkey: String,
    pub output_dir: PathBuf,
    pub last_saved: Option<PathBuf>,
    /// Version of a downloaded update that a restart would install.
    #[serde(default)]
    pub update_ready: Option<String>,
    /// What the updater is doing, for Settings → Updates.
    #[serde(default)]
    pub update: UpdateState,
}

/// The tray's updater, as Settings → Updates shows it. Times are Unix seconds.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum UpdateState {
    /// This build doesn't update itself (dev builds, `PLAYERSTV_INSTANCE`).
    Unavailable,
    /// Not checked yet since the tray started.
    #[default]
    Idle,
    Checking,
    UpToDate {
        checked_at: u64,
    },
    Downloading {
        version: String,
    },
    /// Downloaded and verified; installs on restart.
    Ready {
        version: String,
    },
    Failed {
        error: String,
        at: u64,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Request {
    Status,
    SaveClip,
    ToggleSession,
    /// Quit the tray (and with it the window, see [`QuitSignal`]).
    Quit,
    /// Install the downloaded update and start playerstv again.
    Restart,
    /// Check for an update now (and download it if there is one).
    CheckUpdate,
}

impl Request {
    fn as_str(self) -> &'static str {
        match self {
            Request::Status => "status",
            Request::SaveClip => "clip",
            Request::ToggleSession => "session",
            Request::Quit => "quit",
            Request::Restart => "restart",
            Request::CheckUpdate => "check-update",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        [
            Request::Status,
            Request::SaveClip,
            Request::ToggleSession,
            Request::Quit,
            Request::Restart,
            Request::CheckUpdate,
        ]
        .into_iter()
        .find(|r| r.as_str() == s)
    }
}

/// A named event the tray sets as it quits, so an open window closes with
/// it. A crashed tray never sets it, so the window stays and offers to start
/// the tray again.
pub struct QuitSignal(HANDLE);

// SAFETY: kernel event handles may be used from any thread, and the event
// calls below (set, reset, wait) are thread-safe in Windows itself.
unsafe impl Send for QuitSignal {}
// SAFETY: as for Send; `&self` methods only set, reset or wait on the event.
unsafe impl Sync for QuitSignal {}

// Every `unsafe` below calls Windows with `self.0`, the event handle `open`
// created, which stays valid until `drop` closes it.
impl QuitSignal {
    pub fn open() -> Result<Self> {
        use windows::Win32::System::Threading::CreateEventW;
        // Manual reset: stays set for every waiting window.
        // SAFETY: plain FFI call; the handle it returns is owned by Self.
        let handle = unsafe { CreateEventW(None, true, false, &crate::paths::mutex_name("quit")) }?;
        Ok(Self(handle))
    }

    /// Called by a starting tray: a window from before a quit mustn't close.
    pub fn reset(&self) {
        // SAFETY: see the impl's comment.
        unsafe {
            let _ = windows::Win32::System::Threading::ResetEvent(self.0);
        }
    }

    pub fn set(&self) {
        // SAFETY: see the impl's comment.
        unsafe {
            let _ = windows::Win32::System::Threading::SetEvent(self.0);
        }
    }

    /// Blocks until the tray quits.
    pub fn wait(&self) {
        use windows::Win32::System::Threading::{WaitForSingleObject, INFINITE};
        // SAFETY: see the impl's comment.
        unsafe {
            WaitForSingleObject(self.0, INFINITE);
        }
    }
}

impl Drop for QuitSignal {
    fn drop(&mut self) {
        // SAFETY: we own the handle; this is the only place it's closed.
        unsafe {
            let _ = windows::Win32::Foundation::CloseHandle(self.0);
        }
    }
}

/// Pipe instances kept listening. A single instance leaves a gap between
/// one client disconnecting and the next instance being created, and a
/// client arriving in that gap would see "no such pipe".
const LISTENERS: usize = 4;

/// Runs the server on background threads for the life of the process.
/// `on_request` sees every request, `Status` included (the window polls it
/// while open), before the snapshot is sent back.
pub fn serve(snapshot: Arc<Mutex<Snapshot>>, on_request: impl Fn(Request) + Send + Sync + 'static) {
    let on_request = Arc::new(on_request);
    for i in 0..LISTENERS {
        let snapshot = snapshot.clone();
        let on_request = on_request.clone();
        let spawned = std::thread::Builder::new().name(format!("control-{i}")).spawn(move || loop {
            if let Err(e) = serve_one(&snapshot, &*on_request) {
                tracing::debug!("control connection: {e:#}");
                std::thread::sleep(Duration::from_millis(200));
            }
        });
        if let Err(e) = spawned {
            tracing::error!("couldn't start control listener: {e}");
        }
    }
}

fn serve_one(snapshot: &Mutex<Snapshot>, on_request: &impl Fn(Request)) -> Result<()> {
    // SAFETY: plain FFI call; the handle is checked below.
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
    // SAFETY: `handle` is a valid pipe handle that nothing else owns; from
    // here the File owns it and closes it when dropped.
    let file = unsafe { File::from_raw_handle(handle.0) };
    // Borrowed from `file`, which keeps it open for the calls below.
    let pipe = HANDLE(file.as_raw_handle());
    // SAFETY: `pipe` is open (see above).
    match unsafe { ConnectNamedPipe(pipe, None) } {
        Ok(()) => {}
        Err(e) if e.code() == ERROR_PIPE_CONNECTED.to_hresult() => {}
        Err(e) => return Err(e).context("ConnectNamedPipe"),
    }
    let result = answer(&file, snapshot, on_request);
    // SAFETY: `pipe` is still open: `file` is dropped after this.
    unsafe {
        let _ = FlushFileBuffers(pipe);
        let _ = DisconnectNamedPipe(pipe);
    }
    result
}

fn answer(file: &File, snapshot: &Mutex<Snapshot>, on_request: &impl Fn(Request)) -> Result<()> {
    let mut line = String::new();
    BufReader::new(file).read_line(&mut line)?;
    let request = Request::parse(line.trim()).with_context(|| format!("unknown request {line:?}"))?;
    on_request(request);
    let reply = serde_json::to_string(&*crate::sync::lock(snapshot))?;
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
            // All listeners busy, or between instances while the tray is
            // running: retry briefly rather than drop the user's click.
            Err(e) if is_transient(&e) && attempts < 20 => {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn many_concurrent_clients_all_get_answers() {
        // Own pipe/mutex names so this never talks to a real tray instance.
        std::env::set_var("PLAYERSTV_INSTANCE", format!("unittest-{}", std::process::id()));
        // Hold the tray mutex like the real tray does.
        // SAFETY: plain FFI call; the handle is held until the test process exits.
        let _mutex = unsafe {
            windows::Win32::System::Threading::CreateMutexW(None, false, &crate::paths::mutex_name("tray")).unwrap()
        };
        let snapshot = Arc::new(Mutex::new(Snapshot {
            status: Status::Waiting,
            buffer_seconds: 30,
            clip_hotkey: "Alt+F10".into(),
            session_hotkey: "Alt+F9".into(),
            output_dir: PathBuf::from("out"),
            last_saved: None,
            update_ready: None,
            update: UpdateState::default(),
        }));
        let clips = Arc::new(AtomicUsize::new(0));
        {
            let clips = clips.clone();
            serve(snapshot, move |r| {
                if r == Request::SaveClip {
                    clips.fetch_add(1, Ordering::SeqCst);
                }
            });
        }

        // 8 threads x 25 requests, back to back: with a single listener the
        // gaps between pipe instances made some of these fail.
        let threads: Vec<_> = (0..8)
            .map(|_| {
                std::thread::spawn(|| {
                    (0..25)
                        .map(|i| request(if i % 5 == 0 { Request::SaveClip } else { Request::Status }))
                        .filter(|r| r.is_err())
                        .count()
                })
            })
            .collect();
        let failures: usize = threads.into_iter().map(|t| t.join().unwrap()).sum();
        assert_eq!(failures, 0, "every request should get an answer");
        assert_eq!(clips.load(Ordering::SeqCst), 8 * 5, "every SaveClip should arrive");
    }
}
