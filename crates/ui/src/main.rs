// No console window in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! playerstv-ui: the app window (library, player, clip editor, settings).
//! Started by the tray app; talks to it over the control pipe and the
//! config file (see `ptv_common`).

mod settings;
mod status;
mod window;

fn main() {
    tauri::Builder::default()
        // A second launch (e.g. clicking the tray icon again) focuses this
        // window instead of opening another.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            use tauri::Manager;
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.unminimize();
                let _ = w.show();
                let _ = w.set_focus();
            }
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            window::create_main(app)?;
            status::start_poller(app.handle().clone());
            settings::watch(app.handle().clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            status::get_status,
            status::save_clip,
            status::toggle_session,
            status::start_tray,
            settings::get_config,
            settings::save_config,
            settings::set_game_enabled,
            settings::set_ui_config,
            settings::validate_hotkey,
            settings::storage_usage,
            settings::list_mics,
            settings::pick_folder,
        ])
        .run(tauri::generate_context!())
        .expect("error while running playerstv-ui");
}
