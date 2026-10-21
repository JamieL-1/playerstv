//! Child-process and shell helpers.

use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::{Child, Command};

/// `CreateProcess` flag: don't open a console window for the child.
pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;

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
