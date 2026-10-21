//! Main thread: tray icon, menu, global hotkeys, config hot-reload, toasts and
//! the control channel the playerstv window talks to. Everything else happens
//! on the supervisor thread.

use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use global_hotkey::hotkey::HotKey;
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use notify::{RecursiveMode, Watcher};
use ptv_core::clip::ExportKind;
use ptv_core::config::Config;
use tauri_winrt_notification::Toast;
use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem, Submenu};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, KillTimer, PeekMessageW, PostThreadMessageW, SetTimer, TranslateMessage, MSG,
    PM_NOREMOVE, WM_APP, WM_TIMER,
};

use crate::control::{self, Request, Snapshot};
use crate::supervisor::{self, unix_now, Command, Status, Supervisor};
use crate::{autostart, paths};

const WM_WAKE: u32 = WM_APP + 1;

enum AppEvent {
    Menu(MenuId),
    HotKey(u32),
    OpenWindow,
    Supervisor(supervisor::Event),
    ConfigFileChanged,
}

struct Items {
    status: MenuItem,
    open: MenuItem,
    clip: MenuItem,
    session: MenuItem,
    games: GameToggles,
    open_folder: MenuItem,
    quit: MenuItem,
}

/// "Games ▸" submenu: one checkmark per game, toggling auto-capture.
struct GameToggles {
    submenu: Submenu,
    /// (menu item, game id)
    items: Vec<(CheckMenuItem, String)>,
}

impl GameToggles {
    fn new(cfg: &Config) -> Self {
        let mut t = Self { submenu: Submenu::new("Games to record", true), items: Vec::new() };
        t.rebuild(cfg);
        t
    }

    /// Re-creates the checkmarks from the config (games may have been added,
    /// removed or toggled elsewhere).
    fn rebuild(&mut self, cfg: &Config) {
        for (item, _) in self.items.drain(..) {
            let _ = self.submenu.remove(&item);
        }
        for g in &cfg.games {
            let item = CheckMenuItem::new(&g.name, true, g.enabled, None);
            let _ = self.submenu.append(&item);
            self.items.push((item, g.id.clone()));
        }
    }

    /// The game whose checkmark was clicked, with its new state.
    fn clicked(&self, id: &MenuId) -> Option<(String, bool)> {
        self.items.iter().find(|(item, _)| item.id() == id).map(|(item, game)| (game.clone(), item.is_checked()))
    }
}

struct HotKeys {
    manager: GlobalHotKeyManager,
    clip: Option<HotKey>,
    session: Option<HotKey>,
}

impl HotKeys {
    fn register(&mut self, cfg: &Config) {
        for hk in [self.clip.take(), self.session.take()].into_iter().flatten() {
            let _ = self.manager.unregister(hk);
        }
        self.clip = self.register_one(&cfg.clip_hotkey);
        self.session = self.register_one(&cfg.session_hotkey);
    }

    fn register_one(&self, text: &str) -> Option<HotKey> {
        if text.trim().is_empty() {
            return None;
        }
        match HotKey::from_str(text) {
            Ok(hk) => match self.manager.register(hk) {
                Ok(()) => Some(hk),
                Err(e) => {
                    tracing::warn!("hotkey {text} unavailable: {e}");
                    None
                }
            },
            Err(e) => {
                tracing::warn!("invalid hotkey {text:?}: {e}");
                None
            }
        }
    }
}

pub fn run(mut cfg: Config) -> Result<()> {
    let thread_id = unsafe { GetCurrentThreadId() };
    // Make sure this thread has a message queue before anyone posts to it.
    unsafe {
        let mut msg = MSG::default();
        let _ = PeekMessageW(&mut msg, None, 0, 0, PM_NOREMOVE);
    }
    let (tx, rx) = crossbeam_channel::unbounded::<AppEvent>();
    let post = {
        let tx = tx.clone();
        Arc::new(move |e: AppEvent| {
            let _ = tx.send(e);
            unsafe {
                let _ = PostThreadMessageW(thread_id, WM_WAKE, Default::default(), Default::default());
            }
        })
    };

    {
        let post = post.clone();
        MenuEvent::set_event_handler(Some(move |e: MenuEvent| post(AppEvent::Menu(e.id))));
    }
    {
        let post = post.clone();
        GlobalHotKeyEvent::set_event_handler(Some(move |e: GlobalHotKeyEvent| {
            if e.state == HotKeyState::Pressed {
                post(AppEvent::HotKey(e.id));
            }
        }));
    }
    {
        // Left-click opens the window; right-click shows the menu.
        let post = post.clone();
        TrayIconEvent::set_event_handler(Some(move |e: TrayIconEvent| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = e {
                post(AppEvent::OpenWindow);
            }
        }));
    }

    // Supervisor thread.
    let (cmd_tx, cmd_rx) = crossbeam_channel::unbounded::<Command>();
    let sink: supervisor::EventSink = {
        let post = post.clone();
        Arc::new(move |e| post(AppEvent::Supervisor(e)))
    };
    let sup = Supervisor::new(cfg.clone(), paths::ffmpeg_exe(), sink);
    let sup_thread = std::thread::Builder::new().name("supervisor".into()).spawn(move || sup.run(cmd_rx))?;

    // The playerstv window reads status and sends commands over this.
    let snapshot = Arc::new(Mutex::new(snapshot_of(&cfg, &Status::Waiting, None)));
    {
        let cmd_tx = cmd_tx.clone();
        control::serve(snapshot.clone(), move |req| {
            let _ = cmd_tx.send(match req {
                Request::SaveClip => Command::SaveClip,
                Request::ToggleSession => Command::ToggleSession,
                Request::Status => return,
            });
        });
    }

    // Config hot-reload: the settings window writes the file, we pick it up.
    let config_path = paths::config_file();
    let _watcher = {
        let post = post.clone();
        let file_name = config_path.file_name().map(|n| n.to_owned());
        let mut w = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            if let Ok(ev) = res {
                if ev.paths.iter().any(|p| p.file_name() == file_name.as_deref()) {
                    post(AppEvent::ConfigFileChanged);
                }
            }
        })?;
        w.watch(config_path.parent().unwrap_or(Path::new(".")), RecursiveMode::NonRecursive)?;
        w
    };

    let mut items = Items {
        status: MenuItem::new("Waiting for a game…", false, None),
        open: MenuItem::new("Open playerstv", true, None),
        clip: MenuItem::new("", true, None),
        session: MenuItem::new("", true, None),
        games: GameToggles::new(&cfg),
        open_folder: MenuItem::new("Open clips folder", true, None),
        quit: MenuItem::new("Quit", true, None),
    };
    let menu = Menu::new();
    menu.append_items(&[
        &items.status,
        &PredefinedMenuItem::separator(),
        &items.open,
        &items.clip,
        &items.session,
        &items.games.submenu,
        &items.open_folder,
        &PredefinedMenuItem::separator(),
        &items.quit,
    ])?;
    let tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(false)
        .with_tooltip(tooltip(&Status::Waiting, &cfg))
        .with_icon(status_icon(&Status::Waiting))
        .build()
        .context("creating tray icon")?;

    let mut hotkeys = HotKeys { manager: GlobalHotKeyManager::new()?, clip: None, session: None };
    hotkeys.register(&cfg);
    update_labels(&items, &cfg, &Status::Waiting);
    apply_autostart(&cfg);

    // Ticks once a second to keep the "REC 12:34" labels current.
    let timer = unsafe { SetTimer(None, 0, 1000, None) };

    let mut status = Status::Waiting;
    let mut last_tooltip = String::new();
    let mut last_saved: Option<PathBuf> = None;

    let mut msg = MSG::default();
    'outer: while unsafe { GetMessageW(&mut msg, None, 0, 0) }.as_bool() {
        if msg.message == WM_TIMER && msg.hwnd.is_invalid() {
            // Both the session timer and "Ns of 30s ready" change over time;
            // only touch the menu when the text actually changed.
            let line = tooltip(&status, &cfg);
            if status.is_recording() && line != last_tooltip {
                update_labels(&items, &cfg, &status);
                let _ = tray.set_tooltip(Some(&line));
                last_tooltip = line;
            }
            continue;
        }
        if msg.message != WM_WAKE {
            unsafe {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            continue;
        }
        while let Ok(event) = rx.try_recv() {
            match event {
                AppEvent::HotKey(id) => {
                    if hotkeys.clip.is_some_and(|h| h.id() == id) {
                        let _ = cmd_tx.send(Command::SaveClip);
                    } else if hotkeys.session.is_some_and(|h| h.id() == id) {
                        let _ = cmd_tx.send(Command::ToggleSession);
                    }
                }
                AppEvent::OpenWindow => open_window(),
                AppEvent::Menu(id) => {
                    if id == *items.clip.id() {
                        let _ = cmd_tx.send(Command::SaveClip);
                    } else if id == *items.session.id() {
                        let _ = cmd_tx.send(Command::ToggleSession);
                    } else if id == *items.open.id() {
                        open_window();
                    } else if id == *items.open_folder.id() {
                        let _ = std::fs::create_dir_all(&cfg.output_dir);
                        let _ = std::process::Command::new("explorer").arg(&cfg.output_dir).spawn();
                    } else if id == *items.quit.id() {
                        break 'outer;
                    } else if let Some((game, enabled)) = items.games.clicked(&id) {
                        // Written to disk; the config watcher applies it like
                        // any other settings change.
                        let mut disk = Config::load_or_create(&config_path).unwrap_or_else(|_| cfg.clone());
                        if disk.set_game_enabled(&game, enabled) {
                            if let Err(e) = disk.save(&config_path) {
                                tracing::warn!("couldn't save game toggle: {e:#}");
                            }
                        }
                    }
                }
                AppEvent::Supervisor(supervisor::Event::Status(s)) => {
                    if cfg.notifications {
                        match (status.session_since(), s.session_since()) {
                            (None, Some(_)) => toast(
                                "● Recording full session",
                                &format!("Stop it with {} or from the playerstv window.", cfg.session_hotkey),
                            ),
                            (Some(_), None) => toast("Session stopped", "Saving…"),
                            _ => {}
                        }
                    }
                    let _ = tray.set_icon(Some(status_icon(&s)));
                    let _ = tray.set_tooltip(Some(tooltip(&s, &cfg)));
                    update_labels(&items, &cfg, &s);
                    status = s;
                }
                AppEvent::Supervisor(supervisor::Event::Saved { path, kind }) => {
                    if cfg.notifications {
                        let title = match kind {
                            ExportKind::Clip => format!("Clip saved ({}s)", cfg.buffer_seconds),
                            ExportKind::Session => "Session saved".to_owned(),
                        };
                        toast(&title, &path.file_name().unwrap_or_default().to_string_lossy());
                    }
                    last_saved = Some(path);
                }
                AppEvent::Supervisor(supervisor::Event::Cleaned { count, bytes }) => {
                    if cfg.notifications {
                        let plural = if count == 1 { "" } else { "s" };
                        toast(
                            "Storage limit reached",
                            &format!(
                                "Deleted {count} old recording{plural} ({}) to stay under {} GB.",
                                ptv_core::storage::format_size(bytes),
                                cfg.storage_limit_gb.unwrap_or_default()
                            ),
                        );
                    }
                }
                AppEvent::Supervisor(supervisor::Event::Error(e)) => {
                    tracing::warn!("{e}");
                    if cfg.notifications {
                        toast("playerstv", &e);
                    }
                }
                AppEvent::ConfigFileChanged => {
                    // One save fires several watcher events. Reloading is
                    // cheap and a no-op when nothing changed, so every event
                    // reloads; skipping some could lose a quick second change.
                    match Config::load_or_create(&config_path) {
                        Ok(new) if new != cfg => {
                            tracing::info!("config reloaded");
                            if new.games != cfg.games {
                                items.games.rebuild(&new);
                            }
                            if new.clip_hotkey != cfg.clip_hotkey || new.session_hotkey != cfg.session_hotkey {
                                hotkeys.register(&new);
                            }
                            if new.start_with_windows != cfg.start_with_windows {
                                apply_autostart(&new);
                            }
                            cfg = new;
                            update_labels(&items, &cfg, &status);
                            let _ = cmd_tx.send(Command::Reload(Box::new(cfg.clone())));
                        }
                        Ok(_) => {}
                        Err(e) => tracing::warn!("config not reloaded: {e:#}"),
                    }
                }
            }
        }
        *snapshot.lock().unwrap() = snapshot_of(&cfg, &status, last_saved.clone());
    }

    tracing::info!("shutting down");
    unsafe {
        let _ = KillTimer(None, timer);
    }
    let _ = tray.set_tooltip(Some("playerstv — saving and shutting down…"));
    let _ = cmd_tx.send(Command::Shutdown);
    let _ = sup_thread.join();
    drop(tray);
    Ok(())
}

fn snapshot_of(cfg: &Config, status: &Status, last_saved: Option<PathBuf>) -> Snapshot {
    Snapshot {
        status: status.clone(),
        buffer_seconds: cfg.buffer_seconds,
        clip_hotkey: cfg.clip_hotkey.clone(),
        session_hotkey: cfg.session_hotkey.clone(),
        output_dir: cfg.output_dir.clone(),
        last_saved,
    }
}

/// "1:02:03" or "12:34".
pub fn format_elapsed(secs: u64) -> String {
    let (h, m, s) = (secs / 3600, secs / 60 % 60, secs % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m:02}:{s:02}")
    }
}

/// One-line description of the current state, e.g. "● REC 12:34 — RuneLite session".
pub fn status_line(status: &Status, buffer_seconds: u32) -> String {
    match status {
        Status::Waiting => "Waiting for a game…".into(),
        Status::Recording { game, session_since: Some(t), .. } => {
            format!("● REC {} — {game} full session", format_elapsed(unix_now().saturating_sub(*t)))
        }
        Status::Recording { game, since, session_since: None, .. } => {
            let ready = unix_now().saturating_sub(*since).min(buffer_seconds as u64);
            if ready >= buffer_seconds as u64 {
                format!("Replay buffer on — {game} (last {buffer_seconds}s ready)")
            } else {
                format!("Replay buffer on — {game} ({ready}s of {buffer_seconds}s ready)")
            }
        }
        Status::Failing { game, .. } => format!("{game}: encoder error (see log)"),
    }
}

fn update_labels(items: &Items, cfg: &Config, status: &Status) {
    let hk = |s: &str| if s.trim().is_empty() { String::new() } else { format!("\t{s}") };
    items.status.set_text(status_line(status, cfg.buffer_seconds));
    items.clip.set_text(format!("Save last {}s{}", cfg.buffer_seconds, hk(&cfg.clip_hotkey)));
    items.clip.set_enabled(status.is_recording());
    let label = match status.session_since() {
        Some(t) => format!("■ Stop && save session ({})", format_elapsed(unix_now().saturating_sub(t))),
        None => "● Start full-session recording".to_owned(),
    };
    items.session.set_text(format!("{label}{}", hk(&cfg.session_hotkey)));
    items.session.set_enabled(status.is_recording());
}

fn tooltip(status: &Status, cfg: &Config) -> String {
    format!("playerstv — {}", status_line(status, cfg.buffer_seconds))
}

/// A plain coloured dot: grey = idle, green = buffering, red = full session,
/// amber = error. Drawn at runtime so there are no asset files.
fn status_icon(status: &Status) -> Icon {
    let (r, g, b) = match status {
        Status::Waiting => (140, 140, 140),
        Status::Recording { session_since: None, .. } => (40, 190, 90),
        Status::Recording { session_since: Some(_), .. } => (225, 45, 45),
        Status::Failing { .. } => (240, 170, 30),
    };
    const N: u32 = 32;
    let mut rgba = Vec::with_capacity((N * N * 4) as usize);
    let c = (N as f32 - 1.0) / 2.0;
    for y in 0..N {
        for x in 0..N {
            let d = ((x as f32 - c).powi(2) + (y as f32 - c).powi(2)).sqrt();
            let outer = c - 1.0;
            let alpha = (outer - d + 0.5).clamp(0.0, 1.0);
            // Thin dark ring keeps the dot visible on light taskbars.
            let ring = d > outer - 2.5;
            let (pr, pg, pb) = if ring { (r / 2, g / 2, b / 2) } else { (r, g, b) };
            rgba.extend_from_slice(&[pr, pg, pb, (alpha * 255.0) as u8]);
        }
    }
    Icon::from_rgba(rgba, N, N).expect("valid icon")
}

fn toast(title: &str, text: &str) {
    if let Err(e) = Toast::new(Toast::POWERSHELL_APP_ID).title(title).text1(text).show() {
        tracing::debug!("toast failed: {e}");
    }
}

fn open_window() {
    match std::env::current_exe() {
        Ok(exe) => {
            if let Err(e) = std::process::Command::new(exe)
                .arg("--settings")
                .creation_flags(crate::recorder::CREATE_NO_WINDOW)
                .spawn()
            {
                tracing::error!("couldn't open window: {e}");
            }
        }
        Err(e) => tracing::error!("couldn't find own exe: {e}"),
    }
}

fn apply_autostart(cfg: &Config) {
    if let Err(e) = autostart::apply(cfg.start_with_windows) {
        tracing::warn!("couldn't update autostart: {e:#}");
    }
}
