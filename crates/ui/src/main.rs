// No console window in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! playerstv.exe. Without arguments it runs the tray (crates/app), which
//! opens the window by starting this exe again with `--window`. That process
//! is the app window (library, player, clip editor, settings) and talks to
//! the tray over the control pipe and the config file (see `ptv_common`).

mod icons;
mod library;
mod settings;
mod status;
mod thumbs;
mod trim;
mod window;

fn main() {
    #[cfg(feature = "bundle-ffmpeg")]
    if let Err(e) =
        ptv_common::paths::install_bundled_ffmpeg(include_bytes!(env!("PTV_FFMPEG_PATH")), env!("PTV_FFMPEG_KEY"))
    {
        // Logging isn't up yet; the tray logs which ffmpeg it ends up using.
        eprintln!("playerstv: couldn't unpack ffmpeg: {e}");
    }
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some(playerstv_tray::WINDOW_FLAG) {
        run_window();
    } else {
        playerstv_tray::main(&args);
    }
}

fn run_window() {
    if !window::webview2_available() {
        window::explain_missing_webview2();
        return;
    }
    tauri::Builder::default()
        .manage(trim::Exports::default())
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
            status::close_with_tray(app.handle().clone());
            library::init(app.handle());
            settings::watch(app.handle().clone());
            Ok(())
        })
        .on_window_event(|_, event| {
            if let tauri::WindowEvent::CloseRequested { .. } = event {
                // Closing the window quits playerstv, tray included. Sent
                // before the window goes, so the process can't exit first.
                let _ = ptv_common::control::request(ptv_common::control::Request::Quit);
            }
        })
        .invoke_handler(tauri::generate_handler![
            status::quit_app,
            status::get_status,
            status::save_clip,
            status::toggle_session,
            status::restart_to_update,
            status::start_tray,
            settings::get_config,
            settings::save_config,
            settings::default_config,
            settings::set_game_enabled,
            settings::set_ui_config,
            settings::validate_hotkey,
            settings::storage_usage,
            settings::list_mics,
            settings::pick_folder,
            icons::game_icons,
            icons::add_game,
            library::list_recordings,
            library::delete_recording,
            library::show_in_explorer,
            library::open_external,
            library::open_output_folder,
            trim::export_trim,
            trim::cancel_trim,
        ])
        .run(tauri::generate_context!())
        .expect("error while running the playerstv window");
}
