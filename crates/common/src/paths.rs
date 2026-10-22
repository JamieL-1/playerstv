use std::path::{Path, PathBuf};
use std::sync::OnceLock;

pub const APP_NAME: &str = "playerstv";

fn env_dir(var: &str) -> PathBuf {
    std::env::var_os(var).map(PathBuf::from).unwrap_or_else(std::env::temp_dir).join(APP_NAME)
}

/// `%APPDATA%\playerstv\config.toml`, or `PLAYERSTV_CONFIG` if set.
pub fn config_file() -> PathBuf {
    std::env::var_os("PLAYERSTV_CONFIG").map(PathBuf::from).unwrap_or_else(|| env_dir("APPDATA").join("config.toml"))
}

/// `PLAYERSTV_INSTANCE=name` runs a fully separate instance (its own
/// single-instance locks, control pipe, buffer and logs), so a test or dev
/// build can run next to the real one without touching it.
pub fn instance() -> Option<String> {
    std::env::var("PLAYERSTV_INSTANCE")
        .ok()
        .map(|s| s.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').collect::<String>())
        .filter(|s| !s.is_empty())
}

/// `base` for the default instance, `base-<instance>` otherwise.
pub fn instance_name(base: &str) -> String {
    match instance() {
        Some(i) => format!("{base}-{i}"),
        None => base.to_owned(),
    }
}

/// `%LOCALAPPDATA%\playerstv\buffer` — replay-buffer segments live here.
pub fn buffer_root() -> PathBuf {
    env_dir("LOCALAPPDATA").join(instance_name("buffer"))
}

/// The app window's metadata cache for the library.
pub fn library_cache() -> PathBuf {
    env_dir("LOCALAPPDATA").join(format!("{}.json", instance_name("library")))
}

/// The app window's thumbnail cache.
pub fn thumbs_dir() -> PathBuf {
    env_dir("LOCALAPPDATA").join(instance_name("thumbs"))
}

/// Exists once the window has explained (on its first close) that
/// playerstv keeps running in the tray.
pub fn close_hint_marker() -> PathBuf {
    env_dir("LOCALAPPDATA").join(instance_name("close-hint-shown"))
}

pub fn log_dir() -> PathBuf {
    env_dir("LOCALAPPDATA").join(instance_name("logs"))
}

/// Name for a named kernel object (single-instance mutexes, the quit event),
/// e.g. `Local\playerstv-tray`.
pub fn mutex_name(what: &str) -> windows::core::HSTRING {
    windows::core::HSTRING::from(format!("Local\\{}", instance_name(&format!("playerstv-{what}"))))
}

/// The tray's control pipe (see `control.rs`).
pub fn control_pipe() -> String {
    format!(r"\\.\pipe\{}", instance_name("playerstv-control"))
}

static BUNDLED_FFMPEG: OnceLock<PathBuf> = OnceLock::new();

/// Unpacks the ffmpeg.exe built into release exes to
/// `%LOCALAPPDATA%\playerstv\ffmpeg\<key>\ffmpeg.exe` (once per ffmpeg build;
/// older copies are removed) and makes [`ffmpeg_exe`] use it.
pub fn install_bundled_ffmpeg(bytes: &[u8], key: &str) -> std::io::Result<()> {
    let exe = unpack_ffmpeg(&env_dir("LOCALAPPDATA").join("ffmpeg"), bytes, key)?;
    let _ = BUNDLED_FFMPEG.set(exe);
    Ok(())
}

fn unpack_ffmpeg(root: &Path, bytes: &[u8], key: &str) -> std::io::Result<PathBuf> {
    let exe = root.join(key).join("ffmpeg.exe");
    let current = |exe: &Path| std::fs::metadata(exe).is_ok_and(|m| m.len() == bytes.len() as u64);
    if !current(&exe) {
        std::fs::create_dir_all(root.join(key))?;
        let part = exe.with_extension(format!("{}.part", std::process::id()));
        std::fs::write(&part, bytes)?;
        if let Err(e) = std::fs::rename(&part, &exe) {
            let _ = std::fs::remove_file(&part);
            // The window process may have unpacked it at the same moment.
            if !current(&exe) {
                return Err(e);
            }
        }
        if let Ok(entries) = std::fs::read_dir(root) {
            for old in entries.flatten().filter(|e| e.file_name() != key) {
                // Fails harmlessly while an older version is still running.
                let _ = std::fs::remove_dir_all(old.path());
            }
        }
    }
    Ok(exe)
}

/// Finds ffmpeg.exe: next to our exe (lets a user swap in their own), then
/// the copy unpacked from the exe, then a `vendor/ffmpeg` folder above it
/// (dev builds under `target/`), then PATH.
pub fn ffmpeg_exe() -> PathBuf {
    let exe = std::env::current_exe().unwrap_or_default();
    let dir = exe.parent().unwrap_or(Path::new("."));
    let beside = dir.join("ffmpeg.exe");
    if beside.exists() {
        return beside;
    }
    if let Some(bundled) = BUNDLED_FFMPEG.get() {
        return bundled.clone();
    }
    for ancestor in dir.ancestors() {
        let vendored = ancestor.join("vendor").join("ffmpeg").join("ffmpeg.exe");
        if vendored.exists() {
            return vendored;
        }
    }
    PathBuf::from("ffmpeg.exe")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unpacks_ffmpeg_once_and_removes_old_versions() {
        let root = std::env::temp_dir().join(format!("ptv-ffmpeg-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("oldkey")).unwrap();
        std::fs::write(root.join("oldkey").join("ffmpeg.exe"), b"old").unwrap();

        let exe = unpack_ffmpeg(&root, b"new ffmpeg", "newkey").unwrap();
        assert_eq!(exe, root.join("newkey").join("ffmpeg.exe"));
        assert_eq!(std::fs::read(&exe).unwrap(), b"new ffmpeg");
        assert!(!root.join("oldkey").exists(), "older versions are removed");

        // Already there: left alone (not rewritten on every start).
        let mtime = std::fs::metadata(&exe).unwrap().modified().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        unpack_ffmpeg(&root, b"new ffmpeg", "newkey").unwrap();
        assert_eq!(std::fs::metadata(&exe).unwrap().modified().unwrap(), mtime);

        // A truncated copy (e.g. a crash mid-write) is replaced.
        std::fs::write(&exe, b"new").unwrap();
        unpack_ffmpeg(&root, b"new ffmpeg", "newkey").unwrap();
        assert_eq!(std::fs::read(&exe).unwrap(), b"new ffmpeg");
        assert_eq!(std::fs::read_dir(root.join("newkey")).unwrap().count(), 1, "no .part files left");

        std::fs::remove_dir_all(&root).unwrap();
    }
}
