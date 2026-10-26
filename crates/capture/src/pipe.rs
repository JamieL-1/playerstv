//! Minimal outbound named-pipe server that ffmpeg opens as a file input.

use anyhow::{bail, Context, Result};
use windows::core::HSTRING;
use windows::Win32::Foundation::{CloseHandle, ERROR_PIPE_CONNECTED, GENERIC_READ, HANDLE};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, WriteFile, FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_NONE, OPEN_EXISTING, PIPE_ACCESS_OUTBOUND,
};
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_TYPE_BYTE, PIPE_WAIT,
};

const PIPE_BUFFER: u32 = 1 << 20;

pub struct PipeServer {
    handle: HANDLE,
    name: String,
}

// SAFETY: a pipe handle may be used from any thread, and PipeServer isn't
// Sync: it moves to its feeder thread and is only used there.
unsafe impl Send for PipeServer {}

impl PipeServer {
    pub fn create(name: &str) -> Result<Self> {
        // SAFETY: plain FFI call; the name is a valid wide string for its
        // duration, and the returned handle is checked below.
        let handle = unsafe {
            CreateNamedPipeW(
                &HSTRING::from(name),
                PIPE_ACCESS_OUTBOUND,
                PIPE_TYPE_BYTE | PIPE_WAIT,
                1,
                PIPE_BUFFER,
                0,
                0,
                None,
            )
        };
        if handle.is_invalid() {
            bail!("CreateNamedPipe({name}) failed: {}", windows::core::Error::from_thread());
        }
        Ok(Self { handle, name: name.to_owned() })
    }

    /// Blocks until a client (ffmpeg) opens the pipe.
    pub fn wait_for_client(&self) -> Result<()> {
        // SAFETY: `self.handle` is the live pipe from `create`, closed only on drop.
        match unsafe { ConnectNamedPipe(self.handle, None) } {
            Ok(()) => Ok(()),
            Err(e) if e.code() == ERROR_PIPE_CONNECTED.to_hresult() => Ok(()),
            Err(e) => Err(e).with_context(|| format!("connecting {}", self.name)),
        }
    }

    pub fn write_all(&self, mut data: &[u8]) -> Result<()> {
        while !data.is_empty() {
            let mut written = 0u32;
            // SAFETY: the handle is live (see above); `data` and `written`
            // outlive this synchronous (non-overlapped) write.
            unsafe { WriteFile(self.handle, Some(data), Some(&mut written), None) }
                .with_context(|| format!("writing {}", self.name))?;
            data = &data[written as usize..];
        }
        Ok(())
    }
}

impl Drop for PipeServer {
    fn drop(&mut self) {
        // SAFETY: we own the handle; this is the only place it's closed.
        unsafe {
            let _ = DisconnectNamedPipe(self.handle);
            let _ = CloseHandle(self.handle);
        }
    }
}

/// Opens and immediately closes the pipe as a client, releasing a server
/// thread stuck in [`PipeServer::wait_for_client`] (e.g. ffmpeg died first).
pub fn poke(name: &str) {
    // SAFETY: plain FFI calls; a handle we open is closed straight away.
    unsafe {
        if let Ok(h) = CreateFileW(
            &HSTRING::from(name),
            GENERIC_READ.0,
            FILE_SHARE_NONE,
            None,
            OPEN_EXISTING,
            FILE_FLAGS_AND_ATTRIBUTES(0),
            None,
        ) {
            let _ = CloseHandle(h);
        }
    }
}
