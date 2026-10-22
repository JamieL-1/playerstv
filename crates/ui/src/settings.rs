//! Reading and writing config.toml. The tray watches the same file and
//! applies changes as soon as they're saved.

use std::str::FromStr;
use std::time::Duration;

use global_hotkey::hotkey::HotKey;
use notify::{RecursiveMode, Watcher};
use ptv_common::paths;
use ptv_core::config::{Config, UiConfig};
use tauri::{AppHandle, Emitter};
use tauri_plugin_dialog::DialogExt;

fn load() -> Result<Config, String> {
    Config::load_or_create(&paths::config_file()).map_err(|e| format!("{e:#}"))
}

fn save(cfg: &Config) -> Result<(), String> {
    cfg.save(&paths::config_file()).map_err(|e| format!("{e:#}"))
}

/// The same checks the tray relies on, so a saved config always loads there.
fn validate(cfg: &Config) -> Result<(), String> {
    for (label, hk) in [("Save clip", &cfg.clip_hotkey), ("Session", &cfg.session_hotkey)] {
        if !hk.trim().is_empty() && HotKey::from_str(hk).is_err() {
            return Err(format!("{label} hotkey \"{hk}\" isn't valid (try e.g. Alt+F10)"));
        }
    }
    if !cfg.clip_hotkey.trim().is_empty() && cfg.clip_hotkey.eq_ignore_ascii_case(&cfg.session_hotkey) {
        return Err("Save clip and Session can't use the same hotkey".into());
    }
    for g in &cfg.games {
        if g.name.trim().is_empty() {
            return Err("Every game needs a name".into());
        }
        if g.matcher().is_none() {
            return Err(format!("{}: window title pattern isn't a valid regex", g.name));
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn get_config() -> Result<Config, String> {
    load()
}

/// Validates, normalises and saves the whole config; returns what was saved.
#[tauri::command]
pub async fn save_config(mut config: Config) -> Result<Config, String> {
    validate(&config)?;
    config.sanitize();
    save(&config)?;
    Ok(config)
}

/// One game's on/off switch, written straight away without touching any
/// other unsaved edits in the window.
#[tauri::command]
pub async fn set_game_enabled(id: String, enabled: bool) -> Result<(), String> {
    let mut cfg = load()?;
    if !cfg.set_game_enabled(&id, enabled) {
        return Err(format!("no game with id {id}"));
    }
    save(&cfg)
}

/// Appearance applies instantly, like game switches.
#[tauri::command]
pub async fn set_ui_config(ui: UiConfig) -> Result<UiConfig, String> {
    let mut cfg = load()?;
    cfg.ui = ui;
    cfg.sanitize();
    save(&cfg)?;
    Ok(cfg.ui)
}

#[derive(serde::Serialize)]
pub struct StorageUsage {
    pub bytes: u64,
    pub files: usize,
}

/// Space used by saved recordings in `dir` (same rules as the storage limit).
#[tauri::command]
pub async fn storage_usage(dir: String) -> StorageUsage {
    tauri::async_runtime::spawn_blocking(move || {
        let recs = ptv_core::storage::scan(std::path::Path::new(&dir));
        StorageUsage { bytes: ptv_core::storage::total_size(&recs), files: recs.len() }
    })
    .await
    .unwrap_or(StorageUsage { bytes: 0, files: 0 })
}

#[tauri::command]
pub async fn validate_hotkey(hotkey: String) -> bool {
    HotKey::from_str(&hotkey).is_ok()
}

#[tauri::command]
pub async fn list_mics() -> Vec<String> {
    ptv_capture::audio::list_mic_devices().unwrap_or_default()
}

#[tauri::command]
pub async fn pick_folder(app: AppHandle, start: Option<String>) -> Option<String> {
    let mut dialog = app.dialog().file();
    if let Some(dir) = start {
        dialog = dialog.set_directory(dir);
    }
    dialog.blocking_pick_folder().and_then(|p| p.into_path().ok()).map(|p| p.display().to_string())
}

/// Emits `config-changed` (with the new config) whenever config.toml
/// changes on disk, e.g. a game toggled from the tray menu.
pub fn watch(app: AppHandle) {
    std::thread::Builder::new()
        .name("config-watch".into())
        .spawn(move || {
            let path = paths::config_file();
            let Some(dir) = path.parent().map(|p| p.to_owned()) else { return };
            let file = path.file_name().map(|f| f.to_owned());
            let (tx, rx) = std::sync::mpsc::channel();
            let Ok(mut watcher) = notify::recommended_watcher(tx) else { return };
            if watcher.watch(&dir, RecursiveMode::NonRecursive).is_err() {
                return;
            }
            let mut last: Option<Config> = None;
            for event in rx.into_iter().flatten() {
                if !event.paths.iter().any(|p| p.file_name() == file.as_deref()) {
                    continue;
                }
                // Saves land as write-then-rename; let it settle.
                std::thread::sleep(Duration::from_millis(50));
                if let Ok(cfg) = load() {
                    if last.as_ref() != Some(&cfg) {
                        let _ = app.emit("config-changed", &cfg);
                        crate::library::config_changed(&app, &cfg);
                        last = Some(cfg);
                    }
                }
            }
        })
        .expect("spawn config watcher");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validation_matches_what_the_tray_accepts() {
        let mut cfg = Config::default();
        assert!(validate(&cfg).is_ok());
        cfg.clip_hotkey = "Alt+Nope".into();
        assert!(validate(&cfg).unwrap_err().contains("Save clip hotkey"));
        cfg.clip_hotkey = "alt+f9".into();
        cfg.session_hotkey = "Alt+F9".into();
        assert!(validate(&cfg).unwrap_err().contains("same hotkey"));
        cfg.clip_hotkey = String::new(); // empty = no hotkey, allowed
        assert!(validate(&cfg).is_ok());
        cfg.games[0].window_title_regex = "(".into();
        assert!(validate(&cfg).unwrap_err().contains("regex"));
    }
}
