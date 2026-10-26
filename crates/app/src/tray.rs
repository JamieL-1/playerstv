//! Main thread: tray icon, menu, global hotkeys, config hot-reload, toasts and
//! the control channel the playerstv window talks to. Everything else happens
//! on the supervisor thread.

use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use crossbeam_channel::{Receiver, Sender};
use global_hotkey::hotkey::HotKey;
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use notify::{RecursiveMode, Watcher};
use ptv_common::sync::lock;
use ptv_core::clip::ExportKind;
use ptv_core::config::Config;
use tauri_winrt_notification::Toast;
use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem, Submenu};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, KillTimer, PeekMessageW, PostThreadMessageW, SetTimer, TranslateMessage, MSG,
    PM_NOREMOVE, WM_APP, WM_TIMER,
};

use crate::control::{self, Request, Snapshot, UpdateState};
use crate::supervisor::{self, unix_now, Command, Status, Supervisor};
use crate::updater::{self, Staged};
use crate::{autostart, paths};

const WM_WAKE: u32 = WM_APP + 1;
/// The window asks for status every 500 ms while it's open.
const WINDOW_POLL_GRACE_SECS: u64 = 3;
/// How long a just-launched window gets to start polling before it counts
/// as closed.
const WINDOW_STARTUP_GRACE: Duration = Duration::from_secs(30);

enum AppEvent {
    Menu(MenuId),
    HotKey(u32),
    OpenWindow,
    /// Quit from the window.
    Quit,
    /// "Restart to update" from the window.
    Restart,
    /// "Check for updates" from the window.
    CheckUpdate,
    Supervisor(supervisor::Event),
    ConfigFileChanged,
    UpdateReady(Staged),
    UpdateState(UpdateState),
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

/// Queues an event for the main thread and wakes its message loop.
type Post = Arc<dyn Fn(AppEvent) + Send + Sync>;

/// Why the message loop ended.
enum Exit {
    Quit,
    /// Start playerstv again with these arguments once this process has exited.
    Restart(&'static [&'static str]),
}

/// Runs the tray until Quit; `show_window` also opens the window once it's up.
pub fn run(cfg: Config, show_window: bool) -> Result<()> {
    let (post, events) = event_queue();
    install_event_handlers(&post);
    let (cmd_tx, sup_thread) = spawn_supervisor(&cfg, &post)?;

    // The playerstv window reads status and sends commands over this.
    let snapshot = Arc::new(Mutex::new(snapshot_of(&cfg, &Status::Waiting, None, None, &UpdateState::Idle)));
    let window_polled = serve_window(&snapshot, &cmd_tx, &post);
    // Set on the way out so the window closes with us.
    let quit_signal = control::QuitSignal::open().context("creating quit event")?;
    quit_signal.reset();

    let mut tray = Tray::new(cfg, cmd_tx, window_polled, &post)?;
    if show_window {
        tray.launch_window();
    }

    // Ticks once a second to keep the "REC 12:34" labels current.
    // SAFETY: a thread timer (no window, no callback): it arrives as
    // WM_TIMER in this thread's loop below and is killed on the way out.
    let timer = unsafe { SetTimer(None, 0, 1000, None) };
    let exit = tray.message_loop(&events, &snapshot);

    tracing::info!("shutting down");
    let Tray { icon, cmd_tx, check_update, mut pending_update, .. } = tray;
    drop(check_update);
    quit_signal.set();
    // SAFETY: `timer` is the thread timer set above, killed only here.
    unsafe {
        let _ = KillTimer(None, timer);
    }
    let _ = icon.set_tooltip(Some("playerstv\nSaving and shutting down…"));
    let _ = cmd_tx.send(Command::Shutdown);
    let _ = sup_thread.join();
    drop(icon);
    match exit {
        Exit::Restart(args) => crate::relaunch_after_exit(args),
        Exit::Quit => {
            install_update(&mut pending_update);
        }
    }
    Ok(())
}

/// The main thread's event queue. Call on the main thread.
fn event_queue() -> (Post, Receiver<AppEvent>) {
    // SAFETY: plain FFI call with no preconditions.
    let thread_id = unsafe { GetCurrentThreadId() };
    // Make sure this thread has a message queue before anyone posts to it.
    // SAFETY: `msg` outlives the call; PM_NOREMOVE leaves the queue as it is.
    unsafe {
        let mut msg = MSG::default();
        let _ = PeekMessageW(&mut msg, None, 0, 0, PM_NOREMOVE);
    }
    let (tx, rx) = crossbeam_channel::unbounded::<AppEvent>();
    let post: Post = Arc::new(move |e: AppEvent| {
        let _ = tx.send(e);
        // SAFETY: posting to a thread id is safe from any thread; if the
        // main thread has gone, the call just fails.
        unsafe {
            let _ = PostThreadMessageW(thread_id, WM_WAKE, Default::default(), Default::default());
        }
    });
    (post, rx)
}

fn install_event_handlers(post: &Post) {
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
}

fn spawn_supervisor(cfg: &Config, post: &Post) -> Result<(Sender<Command>, JoinHandle<()>)> {
    let (cmd_tx, cmd_rx) = crossbeam_channel::unbounded::<Command>();
    let sink: supervisor::EventSink = {
        let post = post.clone();
        Arc::new(move |e| post(AppEvent::Supervisor(e)))
    };
    let sup = Supervisor::new(cfg.clone(), paths::ffmpeg_exe(), sink);
    let thread = std::thread::Builder::new().name("supervisor".into()).spawn(move || sup.run(cmd_rx))?;
    Ok((cmd_tx, thread))
}

/// Answers the window's requests. Returns when the window last polled for
/// status: it's open if that was recent.
fn serve_window(snapshot: &Arc<Mutex<Snapshot>>, cmd_tx: &Sender<Command>, post: &Post) -> Arc<AtomicU64> {
    let window_polled = Arc::new(AtomicU64::new(0));
    let cmd_tx = cmd_tx.clone();
    let post = post.clone();
    let polled = window_polled.clone();
    control::serve(snapshot.clone(), move |req| {
        let _ = cmd_tx.send(match req {
            Request::SaveClip => Command::SaveClip,
            Request::ToggleSession => Command::ToggleSession,
            Request::Quit => return post(AppEvent::Quit),
            Request::Restart => return post(AppEvent::Restart),
            Request::CheckUpdate => return post(AppEvent::CheckUpdate),
            Request::Status => return polled.store(unix_now(), Ordering::Relaxed),
        });
    });
    window_polled
}

/// Config hot-reload: the settings window writes the file, we pick it up.
fn watch_config(config_path: &Path, post: &Post) -> Result<notify::RecommendedWatcher> {
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
    Ok(w)
}

fn build_tray(cfg: &Config) -> Result<(Items, TrayIcon)> {
    let items = Items {
        status: MenuItem::new("Waiting for a game…", false, None),
        open: MenuItem::new("Open playerstv", true, None),
        clip: MenuItem::new("", true, None),
        session: MenuItem::new("", true, None),
        games: GameToggles::new(cfg),
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
    let icon = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(false)
        .with_tooltip(tooltip(&Status::Waiting, cfg))
        .with_icon(status_icon(&Status::Waiting))
        .build()
        .context("creating tray icon")?;
    Ok((items, icon))
}

/// Starts the updater if this build has one. Returns the sender that wakes
/// it for a check now (dropping it ends the updater) and the initial state.
fn start_updater(allowed: &Arc<AtomicBool>, post: &Post) -> (Sender<()>, UpdateState) {
    let (check_update, check_update_rx) = crossbeam_channel::unbounded::<()>();
    if !updater::enabled() {
        return (check_update, UpdateState::Unavailable);
    }
    let report = post.clone();
    let ready = post.clone();
    updater::spawn(
        allowed.clone(),
        check_update_rx,
        move |s| report(AppEvent::UpdateState(s)),
        move |staged| ready(AppEvent::UpdateReady(staged)),
    );
    (check_update, UpdateState::Idle)
}

/// The message loop's state. Lives on the main thread.
struct Tray {
    cfg: Config,
    config_path: PathBuf,
    items: Items,
    icon: TrayIcon,
    hotkeys: HotKeys,
    cmd_tx: Sender<Command>,
    /// Wakes the updater for a check now; dropped (ending it) on the way out.
    check_update: Sender<()>,
    updates_allowed: Arc<AtomicBool>,
    update_state: UpdateState,
    /// A downloaded update: installed when nothing is recording and the window
    /// is closed (then playerstv restarts by itself), on "Restart to update",
    /// or otherwise on the way out, so the next start runs it.
    pending_update: Option<Staged>,
    status: Status,
    last_tooltip: String,
    last_saved: Option<PathBuf>,
    window_polled: Arc<AtomicU64>,
    window_launched: Option<Instant>,
    _watcher: notify::RecommendedWatcher,
}

impl Tray {
    /// Builds the tray icon and menu, registers the hotkeys and starts the
    /// config watcher and the updater.
    fn new(cfg: Config, cmd_tx: Sender<Command>, window_polled: Arc<AtomicU64>, post: &Post) -> Result<Self> {
        let config_path = paths::config_file();
        let watcher = watch_config(&config_path, post)?;
        let (items, icon) = build_tray(&cfg)?;
        let mut hotkeys = HotKeys { manager: GlobalHotKeyManager::new()?, clip: None, session: None };
        hotkeys.register(&cfg);
        update_labels(&items, &cfg, &Status::Waiting);
        apply_autostart(&cfg);
        let updates_allowed = Arc::new(AtomicBool::new(cfg.auto_update));
        let (check_update, update_state) = start_updater(&updates_allowed, post);
        Ok(Self {
            cfg,
            config_path,
            items,
            icon,
            hotkeys,
            cmd_tx,
            check_update,
            updates_allowed,
            update_state,
            pending_update: None,
            status: Status::Waiting,
            last_tooltip: String::new(),
            last_saved: None,
            window_polled,
            window_launched: None,
            _watcher: watcher,
        })
    }

    fn message_loop(&mut self, events: &Receiver<AppEvent>, snapshot: &Mutex<Snapshot>) -> Exit {
        let mut msg = MSG::default();
        // SAFETY: a standard message loop (GetMessageW here, Translate/Dispatch
        // below) on the thread that owns the queue; `msg` outlives each call.
        while unsafe { GetMessageW(&mut msg, None, 0, 0) }.as_bool() {
            if msg.message == WM_TIMER && msg.hwnd.is_invalid() {
                if let Some(exit) = self.on_timer() {
                    return exit;
                }
                continue;
            }
            if msg.message != WM_WAKE {
                // SAFETY: see the loop's comment.
                unsafe {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
                continue;
            }
            while let Ok(event) = events.try_recv() {
                if let Some(exit) = self.handle(event) {
                    return exit;
                }
            }
            *lock(snapshot) = self.snapshot();
        }
        Exit::Quit
    }

    fn on_timer(&mut self) -> Option<Exit> {
        if self.pending_update.is_some()
            && matches!(self.status, Status::Waiting)
            && !self.window_open()
            && install_update(&mut self.pending_update)
        {
            tracing::info!("restarting into the update");
            return Some(Exit::Restart(&[crate::BACKGROUND_FLAG]));
        }
        // Both the session timer and "Ns of 30s ready" change over time;
        // only touch the menu when the text actually changed.
        let line = tooltip(&self.status, &self.cfg);
        if self.status.is_recording() && line != self.last_tooltip {
            update_labels(&self.items, &self.cfg, &self.status);
            let _ = self.icon.set_tooltip(Some(&line));
            self.last_tooltip = line;
        }
        None
    }

    fn handle(&mut self, event: AppEvent) -> Option<Exit> {
        match event {
            AppEvent::HotKey(id) => {
                if self.hotkeys.clip.is_some_and(|h| h.id() == id) {
                    let _ = self.cmd_tx.send(Command::SaveClip);
                } else if self.hotkeys.session.is_some_and(|h| h.id() == id) {
                    let _ = self.cmd_tx.send(Command::ToggleSession);
                }
            }
            AppEvent::OpenWindow => self.launch_window(),
            AppEvent::Quit => return Some(Exit::Quit),
            AppEvent::Restart => return self.restart_to_update(),
            AppEvent::CheckUpdate => {
                if self.update_state != UpdateState::Unavailable && self.check_update.send(()).is_ok() {
                    // Shown straight away; the updater reports from here.
                    self.update_state = UpdateState::Checking;
                }
            }
            AppEvent::UpdateReady(staged) => self.pending_update = Some(staged),
            AppEvent::UpdateState(s) => self.update_state = s,
            AppEvent::Menu(id) => return self.on_menu(&id),
            AppEvent::Supervisor(e) => self.on_supervisor(e),
            AppEvent::ConfigFileChanged => self.reload_config(),
        }
        None
    }

    /// "Restart to update" from the window; ignored with no update pending.
    fn restart_to_update(&mut self) -> Option<Exit> {
        self.pending_update.as_ref()?;
        if install_update(&mut self.pending_update) {
            return Some(Exit::Restart(&[]));
        }
        toast("Update failed", "Couldn't install the update (see the log). playerstv keeps running.");
        self.update_state =
            UpdateState::Failed { error: "Couldn't install the update (see the log)".into(), at: unix_now() };
        None
    }

    fn on_menu(&mut self, id: &MenuId) -> Option<Exit> {
        if id == self.items.clip.id() {
            let _ = self.cmd_tx.send(Command::SaveClip);
        } else if id == self.items.session.id() {
            let _ = self.cmd_tx.send(Command::ToggleSession);
        } else if id == self.items.open.id() {
            self.launch_window();
        } else if id == self.items.open_folder.id() {
            let _ = std::fs::create_dir_all(&self.cfg.output_dir);
            let _ = std::process::Command::new("explorer").arg(&self.cfg.output_dir).spawn();
        } else if id == self.items.quit.id() {
            return Some(Exit::Quit);
        } else if let Some((game, enabled)) = self.items.games.clicked(id) {
            // Written to disk; the config watcher applies it like any other
            // settings change.
            let mut disk = Config::load_or_create(&self.config_path).unwrap_or_else(|_| self.cfg.clone());
            if disk.set_game_enabled(&game, enabled) {
                if let Err(e) = disk.save(&self.config_path) {
                    tracing::warn!("couldn't save game toggle: {e:#}");
                }
            }
        }
        None
    }

    fn on_supervisor(&mut self, event: supervisor::Event) {
        let notify = self.cfg.notifications;
        match event {
            supervisor::Event::Status(s) => self.set_status(s),
            supervisor::Event::Saved { path, kind } => {
                if notify {
                    let title = match kind {
                        ExportKind::Clip => {
                            format!("Clip saved ({}s)", self.status.buffer_seconds_or(self.cfg.buffer_seconds))
                        }
                        ExportKind::Session => "Session saved".to_owned(),
                    };
                    toast(&title, &path.file_name().unwrap_or_default().to_string_lossy());
                }
                self.last_saved = Some(path);
            }
            supervisor::Event::Cleaned { count, bytes } => {
                if notify {
                    let plural = if count == 1 { "" } else { "s" };
                    toast(
                        "Storage limit reached",
                        &format!(
                            "Deleted {count} old recording{plural} ({}) to stay under {} GB.",
                            ptv_core::storage::format_size(bytes),
                            self.cfg.storage_limit_gb.unwrap_or_default()
                        ),
                    );
                }
            }
            supervisor::Event::Error(e) => {
                tracing::warn!("{e}");
                if notify {
                    toast("playerstv", &e);
                }
            }
        }
    }

    fn set_status(&mut self, s: Status) {
        if self.cfg.notifications {
            match (self.status.session_since(), s.session_since()) {
                (None, Some(_)) => toast(
                    "● Recording full session",
                    &format!("Stop it with {} or from the playerstv window.", self.cfg.session_hotkey),
                ),
                (Some(_), None) => toast("Session stopped", "Saving…"),
                _ => {}
            }
        }
        let _ = self.icon.set_icon(Some(status_icon(&s)));
        let _ = self.icon.set_tooltip(Some(tooltip(&s, &self.cfg)));
        update_labels(&self.items, &self.cfg, &s);
        self.status = s;
    }

    fn reload_config(&mut self) {
        // One save fires several watcher events. Reloading is cheap and a
        // no-op when nothing changed, so every event reloads; skipping some
        // could lose a quick second change.
        let new = match Config::load_or_create(&self.config_path) {
            Ok(new) if new != self.cfg => new,
            Ok(_) => return,
            Err(e) => return tracing::warn!("config not reloaded: {e:#}"),
        };
        tracing::info!("config reloaded");
        if new.games != self.cfg.games {
            self.items.games.rebuild(&new);
        }
        if new.clip_hotkey != self.cfg.clip_hotkey || new.session_hotkey != self.cfg.session_hotkey {
            self.hotkeys.register(&new);
        }
        if new.start_with_windows != self.cfg.start_with_windows {
            apply_autostart(&new);
        }
        self.updates_allowed.store(new.auto_update, Ordering::Relaxed);
        if !new.auto_update && self.pending_update.take().is_some() {
            tracing::info!("updates turned off; not installing the downloaded update");
            self.update_state = UpdateState::Idle;
        }
        if new.auto_update && !self.cfg.auto_update {
            // Switched back on: check now, not in hours.
            let _ = self.check_update.send(());
        }
        self.cfg = new;
        update_labels(&self.items, &self.cfg, &self.status);
        let _ = self.cmd_tx.send(Command::Reload(Box::new(self.cfg.clone())));
    }

    fn launch_window(&mut self) {
        open_window();
        self.window_launched = Some(Instant::now());
    }

    fn window_open(&self) -> bool {
        unix_now().saturating_sub(self.window_polled.load(Ordering::Relaxed)) <= WINDOW_POLL_GRACE_SECS
            || self.window_launched.is_some_and(|t| t.elapsed() < WINDOW_STARTUP_GRACE)
    }

    fn snapshot(&self) -> Snapshot {
        snapshot_of(&self.cfg, &self.status, self.last_saved.clone(), self.pending_update.as_ref(), &self.update_state)
    }
}

/// Installs the pending update, if any; true if it was installed. A failed
/// install isn't retried until the next start (which downloads it again).
fn install_update(pending: &mut Option<Staged>) -> bool {
    let Some(staged) = pending.take() else { return false };
    match updater::install(&staged) {
        Ok(()) => true,
        Err(e) => {
            tracing::warn!("couldn't install update {}: {e:#}", staged.version);
            false
        }
    }
}

fn snapshot_of(
    cfg: &Config,
    status: &Status,
    last_saved: Option<PathBuf>,
    update_ready: Option<&Staged>,
    update: &UpdateState,
) -> Snapshot {
    Snapshot {
        status: status.clone(),
        buffer_seconds: status.buffer_seconds_or(cfg.buffer_seconds),
        clip_hotkey: cfg.clip_hotkey.clone(),
        session_hotkey: cfg.session_hotkey.clone(),
        output_dir: cfg.output_dir.clone(),
        last_saved,
        update_ready: update_ready.map(|u| u.version.clone()),
        update: update.clone(),
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

/// One-line description of the current state, e.g. "● REC 12:34 · RuneLite full session".
pub fn status_line(status: &Status, buffer_seconds: u32) -> String {
    match status {
        Status::Waiting => "Waiting for a game…".into(),
        Status::Recording { game, session_since: Some(t), .. } => {
            format!("● REC {} · {game} full session", format_elapsed(unix_now().saturating_sub(*t)))
        }
        Status::Recording { game, since, session_since: None, .. } => {
            let ready = unix_now().saturating_sub(*since).min(buffer_seconds as u64);
            if ready >= buffer_seconds as u64 {
                format!("Replay buffer on · {game} (last {buffer_seconds}s ready)")
            } else {
                format!("Replay buffer on · {game} ({ready}s of {buffer_seconds}s ready)")
            }
        }
        Status::Failing { game, .. } => format!("{game}: encoder error (see log)"),
    }
}

fn update_labels(items: &Items, cfg: &Config, status: &Status) {
    let hk = |s: &str| if s.trim().is_empty() { String::new() } else { format!("\t{s}") };
    let buffer_seconds = status.buffer_seconds_or(cfg.buffer_seconds);
    items.status.set_text(status_line(status, buffer_seconds));
    items.clip.set_text(format!("Save last {buffer_seconds}s{}", hk(&cfg.clip_hotkey)));
    items.clip.set_enabled(status.is_recording());
    let label = match status.session_since() {
        Some(t) => format!("■ Stop && save session ({})", format_elapsed(unix_now().saturating_sub(t))),
        None => "● Start full-session recording".to_owned(),
    };
    items.session.set_text(format!("{label}{}", hk(&cfg.session_hotkey)));
    items.session.set_enabled(status.is_recording());
}

fn tooltip(status: &Status, cfg: &Config) -> String {
    format!("playerstv\n{}", status_line(status, status.buffer_seconds_or(cfg.buffer_seconds)))
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

/// A Windows notification (also used by the window).
pub fn toast(title: &str, text: &str) {
    // Toasts are WinRT calls from threads that never initialise COM, so they
    // run in the process's implicit MTA. That MTA only exists while some
    // thread is in it (the audio feeders); when an encoder restart stops them
    // all, it's torn down and the toast's cached COM factories dangle: the
    // next toast crashed. Holding a usage reference keeps it alive for good.
    static MTA: std::sync::Once = std::sync::Once::new();
    MTA.call_once(|| {
        // SAFETY: plain FFI call; the usage cookie is deliberately never released.
        if let Err(e) = unsafe { windows::Win32::System::Com::CoIncrementMTAUsage() } {
            tracing::warn!("CoIncrementMTAUsage failed: {e}");
        }
    });
    if let Err(e) = Toast::new(Toast::POWERSHELL_APP_ID).title(title).text1(text).show() {
        tracing::warn!("toast failed: {e}");
    }
}

/// Opens the app window: this same exe started with `--window`, in its own
/// process so the window's memory is freed when it closes. If it's already
/// open it focuses itself (single-instance plugin).
pub fn open_window() {
    let ui = match std::env::current_exe() {
        Ok(exe) => exe,
        Err(e) => return tracing::error!("couldn't find own exe: {e}"),
    };
    if let Err(e) = std::process::Command::new(&ui).arg(crate::WINDOW_FLAG).spawn() {
        tracing::error!("couldn't open {}: {e}", ui.display());
        toast("playerstv", &format!("Couldn't open the playerstv window ({}).", ui.display()));
    }
}

fn apply_autostart(cfg: &Config) {
    if let Err(e) = autostart::apply(cfg.start_with_windows) {
        tracing::warn!("couldn't update autostart: {e:#}");
    }
}
