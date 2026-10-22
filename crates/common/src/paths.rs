use std::path::{Path, PathBuf};

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

pub fn log_dir() -> PathBuf {
    env_dir("LOCALAPPDATA").join(instance_name("logs"))
}

/// Named-mutex name for single-instance checks, e.g. `Local\playerstv-tray`.
pub fn mutex_name(what: &str) -> windows::core::HSTRING {
    windows::core::HSTRING::from(format!("Local\\{}", instance_name(&format!("playerstv-{what}"))))
}

/// The tray's control pipe (see `control.rs`).
pub fn control_pipe() -> String {
    format!(r"\\.\pipe\{}", instance_name("playerstv-control"))
}

/// Finds ffmpeg.exe: next to our exe (installed layout), then in a
/// `vendor/ffmpeg` folder above it (dev builds under `target/`), then PATH.
pub fn ffmpeg_exe() -> PathBuf {
    let exe = std::env::current_exe().unwrap_or_default();
    let dir = exe.parent().unwrap_or(Path::new("."));
    let beside = dir.join("ffmpeg.exe");
    if beside.exists() {
        return beside;
    }
    for ancestor in dir.ancestors() {
        let vendored = ancestor.join("vendor").join("ffmpeg").join("ffmpeg.exe");
        if vendored.exists() {
            return vendored;
        }
    }
    PathBuf::from("ffmpeg.exe")
}
