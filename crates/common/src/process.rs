//! Child-process and shell helpers.

use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::{Child, Command};

/// `CreateProcess` flag: don't open a console window for the child.
pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// `CreateProcess` flag: run the child one step above normal CPU priority.
///
/// The capture ffmpeg uses little CPU, but it has to pick each frame up
/// within a frame or two of Windows delivering it (gfxcapture's pool holds
/// two). A game that fills every core at normal priority could otherwise
/// keep it waiting long enough for Windows to drop frames.
pub const ABOVE_NORMAL_PRIORITY_CLASS: u32 = 0x0000_8000;

/// `CreateProcess` flag: run the child one step below normal CPU priority.
pub const BELOW_NORMAL_PRIORITY_CLASS: u32 = 0x0000_4000;

/// Makes the child's disk reads and writes queue behind everyone else's
/// ("low" I/O priority, what Windows gives scheduled tasks), so a big copy
/// doesn't hold up a game's loading or the recording's own writes. It runs
/// at full speed when the disk is otherwise idle.
pub fn lower_io_priority(child: &Child) {
    use std::os::windows::io::AsRawHandle;

    const PROCESS_IO_PRIORITY: u32 = 33; // PROCESSINFOCLASS::ProcessIoPriority
    let io_priority_low: u32 = 1;

    #[link(name = "ntdll")]
    extern "system" {
        fn NtSetInformationProcess(
            process: *mut std::ffi::c_void,
            class: u32,
            info: *const std::ffi::c_void,
            len: u32,
        ) -> i32;
    }

    // SAFETY: `child` keeps its process handle (opened with full access)
    // alive for the call, and `info` points at a u32, the size passed.
    let status = unsafe {
        NtSetInformationProcess(
            child.as_raw_handle(),
            PROCESS_IO_PRIORITY,
            (&raw const io_priority_low).cast(),
            size_of::<u32>() as u32,
        )
    };
    if status < 0 {
        tracing::debug!("couldn't lower a child's I/O priority: NTSTATUS {status:#010x}");
    }
}

/// Asks the GPU scheduler to run process `pid`'s work ahead of
/// normal-priority processes, and returns the class it accepted.
///
/// A game that uses the whole GPU otherwise shares it evenly with the
/// capture's copy/scale and encode submissions, which then fall behind and
/// lose frames. Raising the recorder's GPU priority is what OBS does for the
/// same reason. Only our own process is touched, never the game. The
/// process must already have a graphics device, so call this once it's
/// producing frames.
pub fn raise_gpu_priority(pid: u32) -> Result<&'static str, String> {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{OpenProcess, PROCESS_SET_INFORMATION};

    // D3DKMT_SCHEDULINGPRIORITYCLASS. Realtime (5) needs administrator
    // rights and can starve the desktop, so it isn't tried.
    const CLASSES: [(i32, &str); 2] = [(4, "high"), (3, "above normal")];

    #[link(name = "gdi32")]
    extern "system" {
        fn D3DKMTSetProcessSchedulingPriorityClass(process: *mut std::ffi::c_void, class: i32) -> i32;
    }

    // SAFETY: plain FFI call; the returned handle is checked and closed below.
    let process = unsafe { OpenProcess(PROCESS_SET_INFORMATION, false, pid) }.map_err(|e| e.to_string())?;
    let mut last_status = 0;
    let mut accepted = None;
    for (class, name) in CLASSES {
        // SAFETY: `process` is a live handle opened above with the access
        // this call needs; the function takes it and an enum value.
        last_status = unsafe { D3DKMTSetProcessSchedulingPriorityClass(process.0, class) };
        if last_status >= 0 {
            accepted = Some(name);
            break;
        }
    }
    // SAFETY: we opened `process` and close it exactly once.
    let _ = unsafe { CloseHandle(process) };
    accepted.ok_or_else(|| format!("NTSTATUS {last_status:#010x}"))
}

/// Ties a child process's lifetime to ours: every ffmpeg we start goes into a
/// job object that Windows kills when our last handle to it closes, so a
/// crash or `taskkill /F` of playerstv never leaves ffmpeg recording forever.
pub fn bind_to_our_lifetime(child: &Child) {
    use std::os::windows::io::AsRawHandle;
    use std::sync::OnceLock;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation, SetInformationJobObject,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };

    // HANDLE isn't Send/Sync; store the raw value. Never closed on purpose.
    static JOB: OnceLock<Option<usize>> = OnceLock::new();
    // SAFETY: plain FFI calls; `info` outlives SetInformationJobObject and
    // its size is passed with it.
    let job = JOB.get_or_init(|| unsafe {
        let job = CreateJobObjectW(None, None).ok()?;
        let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            &info as *const _ as *const _,
            std::mem::size_of_val(&info) as u32,
        )
        .ok()?;
        Some(job.0 as usize)
    });
    if let Some(job) = job {
        let process = HANDLE(child.as_raw_handle());
        // SAFETY: the job handle is never closed, and `child` keeps its
        // process handle open for the duration of the call.
        if let Err(e) = unsafe { AssignProcessToJobObject(HANDLE(*job as *mut _), process) } {
            tracing::warn!("couldn't bind child process to our lifetime: {e}");
        }
    }
}

/// Opens Explorer with `path` selected. Explorer only understands
/// `/select,"C:\a b\clip.mp4"`; `Command::arg` would quote the whole
/// argument for a path with spaces (`"/select,C:\a b\…"`), which Explorer
/// ignores and opens Documents instead. Paths can't contain `"`.
pub fn show_in_explorer(path: &Path) {
    let _ = Command::new("explorer").raw_arg(select_arg(path)).spawn();
}

/// Opens a folder in Explorer.
pub fn open_folder(dir: &Path) {
    let _ = std::fs::create_dir_all(dir);
    let _ = Command::new("explorer").arg(dir).spawn();
}

fn select_arg(path: &Path) -> String {
    format!("/select,\"{}\"", path.display())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explorer_select_quotes_only_the_path() {
        let p = Path::new(r"C:\Users\Some User\Videos\playerstv\RuneLite\RuneLite_2026-10-21_14-03-22.mp4");
        assert_eq!(
            select_arg(p),
            r#"/select,"C:\Users\Some User\Videos\playerstv\RuneLite\RuneLite_2026-10-21_14-03-22.mp4""#
        );
    }
}
