//! `playerstv --settings`: the playerstv window. Shows what's being recorded
//! (with Save clip / Start-Stop session buttons) and edits config.toml.
//!
//! Runs as its own short-lived process so the always-on tray process never
//! loads a GUI toolkit. It talks to the tray over the control pipe, and the
//! tray picks up saved settings via a file watcher.

use std::os::windows::process::CommandExt;
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{anyhow, Result};
use eframe::egui;
use global_hotkey::hotkey::HotKey;
use ptv_core::config::{
    AudioSourceKind, Codec, Config, Quality, Resolution, FPS_CHOICES, MAX_BUFFER_SECONDS, MAX_VOLUME_PERCENT,
    MIN_BUFFER_SECONDS,
};
use ptv_core::profile::{self, GameProfile};
use windows::core::w;
use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, SetForegroundWindow, ShowWindow, SW_RESTORE};

use crate::control::{self, Request, Snapshot};
use crate::paths;
use crate::supervisor::{unix_now, Status};
use crate::tray::format_elapsed;

const WINDOW_TITLE: &str = "playerstv";

pub fn run() -> Result<()> {
    unsafe {
        // The handle stays open (and the mutex held) until the process exits.
        let _mutex = CreateMutexW(None, false, &paths::mutex_name("settings"))?;
        if GetLastError() == ERROR_ALREADY_EXISTS {
            // Already open: bring it to the front instead.
            if let Ok(hwnd) = FindWindowW(None, w!("playerstv")) {
                let _ = ShowWindow(hwnd, SW_RESTORE);
                let _ = SetForegroundWindow(hwnd);
            }
            return Ok(());
        }
    }
    let path = paths::config_file();
    let cfg = Config::load_or_create(&path)?;
    let mics = ptv_capture::audio::list_mic_devices().unwrap_or_default();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(WINDOW_TITLE)
            .with_inner_size([560.0, 780.0])
            .with_min_inner_size([460.0, 420.0]),
        ..Default::default()
    };
    eframe::run_native(
        WINDOW_TITLE,
        options,
        Box::new(move |cc| {
            let remote = Remote::start(cc.egui_ctx.clone());
            Ok(Box::new(SettingsApp::new(cfg, mics, remote)))
        }),
    )
    .map_err(|e| anyhow!("window: {e}"))
}

/// Latest status from the tray app, refreshed twice a second.
/// `None` means the tray app isn't running.
#[derive(Clone)]
struct Remote {
    latest: Arc<Mutex<Option<Snapshot>>>,
    ctx: egui::Context,
}

impl Remote {
    fn start(ctx: egui::Context) -> Self {
        let remote = Self { latest: Arc::new(Mutex::new(None)), ctx };
        let r = remote.clone();
        std::thread::spawn(move || loop {
            r.store(control::request(Request::Status).ok());
            std::thread::sleep(Duration::from_millis(500));
        });
        remote
    }

    fn store(&self, snapshot: Option<Snapshot>) {
        *self.latest.lock().unwrap() = snapshot;
        self.ctx.request_repaint();
    }

    fn get(&self) -> Option<Snapshot> {
        self.latest.lock().unwrap().clone()
    }

    /// Sends a command without blocking the UI thread.
    fn send(&self, req: Request) {
        let r = self.clone();
        std::thread::spawn(move || {
            if let Ok(s) = control::request(req) {
                r.store(Some(s));
            }
        });
    }
}

struct SettingsApp {
    saved: Config,
    cfg: Config,
    mics: Vec<String>,
    /// Comma-separated exe names per game, edited as text.
    exe_text: Vec<String>,
    message: Option<(String, bool)>,
    /// Last-seen modification time of config.toml, to notice outside edits.
    config_mtime: Option<std::time::SystemTime>,
    usage: Option<Usage>,
    remote: Remote,
}

impl SettingsApp {
    fn new(cfg: Config, mics: Vec<String>, remote: Remote) -> Self {
        let exe_text = cfg.games.iter().map(|g| g.exe_names.join(", ")).collect();
        Self { saved: cfg.clone(), cfg, mics, exe_text, message: None, remote, config_mtime: config_mtime(), usage: None }
    }

    fn revert(&mut self) {
        self.cfg = self.saved.clone();
        self.exe_text = self.cfg.games.iter().map(|g| g.exe_names.join(", ")).collect();
        self.message = None;
    }

    fn validate(&self) -> Result<(), String> {
        for (label, hk) in [("Clip", &self.cfg.clip_hotkey), ("Session", &self.cfg.session_hotkey)] {
            if !hk.trim().is_empty() && HotKey::from_str(hk).is_err() {
                return Err(format!("{label} hotkey \"{hk}\" isn't valid (try e.g. Alt+F10)"));
            }
        }
        for g in &self.cfg.games {
            if g.name.trim().is_empty() {
                return Err("Every game needs a name".into());
            }
            if g.matcher().is_none() {
                return Err(format!("{}: window title regex is invalid", g.name));
            }
        }
        Ok(())
    }

    fn sync_exe_names(&mut self) {
        for (g, text) in self.cfg.games.iter_mut().zip(&self.exe_text) {
            g.exe_names = text.split(',').map(|s| s.trim().to_owned()).filter(|s| !s.is_empty()).collect();
        }
    }

    fn save(&mut self) {
        if let Err(e) = self.validate() {
            self.message = Some((e, false));
            return;
        }
        self.cfg.sanitize();
        match self.cfg.save(&paths::config_file()) {
            Ok(()) => {
                self.saved = self.cfg.clone();
                self.config_mtime = config_mtime();
                self.message = Some(("Saved — changes apply immediately.".into(), true));
            }
            Err(e) => self.message = Some((format!("Save failed: {e:#}"), false)),
        }
    }
}

impl eframe::App for SettingsApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.reload_if_changed_on_disk();
        self.sync_exe_names();
        egui::Panel::bottom("buttons").show(ui, |ui| {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let dirty = self.cfg != self.saved;
                if ui.add_enabled(dirty, egui::Button::new("Save")).clicked() {
                    self.save();
                }
                if ui.add_enabled(dirty, egui::Button::new("Revert")).clicked() {
                    self.revert();
                }
                if let Some((msg, ok)) = &self.message {
                    let color = if *ok { egui::Color32::from_rgb(60, 170, 90) } else { egui::Color32::from_rgb(220, 70, 60) };
                    ui.colored_label(color, msg);
                }
            });
            ui.add_space(6.0);
        });

        egui::Panel::top("status").show(ui, |ui| {
            ui.add_space(8.0);
            self.status_card(ui);
            ui.add_space(8.0);
        });

        egui::CentralPanel::default().show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                self.general(ui);
                ui.separator();
                self.video(ui);
                ui.separator();
                self.audio(ui);
                ui.separator();
                self.games(ui);
            });
        });
    }
}

const RED: egui::Color32 = egui::Color32::from_rgb(230, 60, 60);
const GREEN: egui::Color32 = egui::Color32::from_rgb(60, 190, 100);
const AMBER: egui::Color32 = egui::Color32::from_rgb(240, 170, 40);
const GREY: egui::Color32 = egui::Color32::from_rgb(140, 140, 140);

impl SettingsApp {
    /// The "is it recording?" card: state, live timer and the main actions.
    fn status_card(&mut self, ui: &mut egui::Ui) {
        let Some(snap) = self.remote.get() else {
            ui.horizontal(|ui| {
                status_dot(ui, GREY, false);
                ui.label(egui::RichText::new("playerstv isn't running — nothing is being recorded").strong());
            });
            if ui.button("Start playerstv").clicked() {
                if let Ok(exe) = std::env::current_exe() {
                    let _ = std::process::Command::new(exe).creation_flags(crate::recorder::CREATE_NO_WINDOW).spawn();
                }
            }
            return;
        };

        let now = unix_now();
        let buffer = snap.buffer_seconds;
        let (color, pulse, title, detail) = match &snap.status {
            Status::Waiting => {
                let enabled: Vec<&str> = self.saved.games.iter().filter(|g| g.enabled).map(|g| g.name.as_str()).collect();
                let detail = match enabled.as_slice() {
                    [] => "Every game is switched off below — nothing will be recorded.".to_owned(),
                    [one] => format!("Recording starts automatically when {one} opens."),
                    [rest @ .., last] => format!("Recording starts automatically when {} or {last} opens.", rest.join(", ")),
                };
                (GREY, false, "Not recording — waiting for a game".to_owned(), detail)
            }
            Status::Recording { game, session_since: Some(_), .. } => (
                RED,
                true,
                format!("Recording full session — {game}"),
                format!("Everything is being saved. The replay buffer (last {buffer}s) is on too."),
            ),
            Status::Recording { game, since, session_since: None, .. } => {
                let ready = now.saturating_sub(*since).min(buffer as u64);
                (
                    GREEN,
                    false,
                    format!("Replay buffer on — {game}"),
                    format!(
                        "Keeping the last {buffer}s ({ready}s ready). Nothing is saved until you clip it ({}).",
                        snap.clip_hotkey
                    ),
                )
            }
            Status::Failing { game, error } => {
                (AMBER, false, format!("{game}: encoder error — retrying"), error.clone())
            }
        };

        ui.horizontal(|ui| {
            status_dot(ui, color, pulse);
            ui.label(egui::RichText::new(title).size(17.0).strong());
            if let Some(t) = snap.status.session_since() {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        egui::RichText::new(format_elapsed(now.saturating_sub(t)))
                            .size(26.0)
                            .monospace()
                            .strong()
                            .color(RED),
                    );
                });
            }
        });
        ui.label(egui::RichText::new(detail).weak());
        if let Status::Recording { video, .. } = &snap.status {
            if !video.is_empty() {
                ui.label(egui::RichText::new(format!("Encoding {video}")).small().weak());
            }
        }
        ui.add_space(6.0);

        let recording = snap.status.is_recording();
        ui.horizontal_wrapped(|ui| {
            let clip = egui::Button::new(format!("Save last {buffer}s  ({})", snap.clip_hotkey));
            if ui.add_enabled(recording, clip).clicked() {
                self.remote.send(Request::SaveClip);
            }
            let session = match snap.status.session_since() {
                Some(_) => egui::Button::new(
                    egui::RichText::new(format!("Stop & save session  ({})", snap.session_hotkey)).color(egui::Color32::WHITE),
                )
                .fill(RED),
                None => egui::Button::new(format!("Record full session  ({})", snap.session_hotkey)),
            };
            if ui.add_enabled(recording, session).clicked() {
                self.remote.send(Request::ToggleSession);
            }
            if ui.button("Open clips folder").clicked() {
                let _ = std::fs::create_dir_all(&snap.output_dir);
                let _ = std::process::Command::new("explorer").arg(&snap.output_dir).spawn();
            }
        });
        if let Some(path) = &snap.last_saved {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Last saved:").weak());
                let name = path.file_name().unwrap_or_default().to_string_lossy();
                if ui.link(name).on_hover_text("Show in Explorer").clicked() {
                    let _ = std::process::Command::new("explorer").arg(format!("/select,{}", path.display())).spawn();
                }
            });
        }
        // Keep the timer and pulsing dot moving between polls.
        ui.ctx().request_repaint_after(Duration::from_millis(if pulse { 50 } else { 500 }));
    }

    fn general(&mut self, ui: &mut egui::Ui) {
        ui.heading("Replay buffer");
        egui::Grid::new("general").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
            ui.label("Clip length");
            ui.add(egui::Slider::new(&mut self.cfg.buffer_seconds, MIN_BUFFER_SECONDS..=MAX_BUFFER_SECONDS).suffix(" s"));
            ui.end_row();

            ui.label("Save clip hotkey");
            ui.text_edit_singleline(&mut self.cfg.clip_hotkey);
            ui.end_row();

            ui.label("Session hotkey");
            ui.text_edit_singleline(&mut self.cfg.session_hotkey);
            ui.end_row();

            ui.label("Output folder");
            ui.horizontal(|ui| {
                let mut text = self.cfg.output_dir.display().to_string();
                if ui.text_edit_singleline(&mut text).changed() {
                    self.cfg.output_dir = text.into();
                }
                if ui.button("Browse…").clicked() {
                    if let Some(dir) = rfd::FileDialog::new().set_directory(&self.cfg.output_dir).pick_folder() {
                        self.cfg.output_dir = dir;
                    }
                }
            });
            ui.end_row();
        });
        ui.checkbox(&mut self.cfg.record_full_session, "Also record the whole session automatically");
        ui.checkbox(&mut self.cfg.start_with_windows, "Start playerstv with Windows");
        ui.checkbox(&mut self.cfg.notifications, "Show a notification when a clip is saved");
        ui.separator();
        self.storage(ui);
    }

    fn storage(&mut self, ui: &mut egui::Ui) {
        ui.heading("Storage");
        let mut limited = self.cfg.storage_limit_gb.is_some();
        if ui.checkbox(&mut limited, "Limit how much space recordings can use").changed() {
            self.cfg.storage_limit_gb = limited.then_some(self.cfg.storage_limit_gb.unwrap_or(50));
        }
        if let Some(gb) = &mut self.cfg.storage_limit_gb {
            ui.indent("storage", |ui| {
                ui.add(egui::Slider::new(gb, 1..=2000).suffix(" GB").logarithmic(true).text("maximum"));
                ui.label(
                    egui::RichText::new(
                        "When saved clips and sessions go over this, the oldest are permanently deleted. \
                         Only files playerstv saved are counted — move a recording out of the folder to keep it.",
                    )
                    .small()
                    .color(AMBER),
                );
            });
        }

        // Current usage, re-measured every few seconds.
        if self.usage.as_ref().is_none_or(|u| u.measured.elapsed() > Duration::from_secs(3) || u.dir != self.cfg.output_dir) {
            let recs = ptv_core::storage::scan(&self.cfg.output_dir);
            self.usage = Some(Usage {
                dir: self.cfg.output_dir.clone(),
                bytes: ptv_core::storage::total_size(&recs),
                files: recs.len(),
                measured: std::time::Instant::now(),
            });
        }
        if let Some(u) = &self.usage {
            let used = ptv_core::storage::format_size(u.bytes);
            let files = match u.files {
                1 => "1 recording".to_owned(),
                n => format!("{n} recordings"),
            };
            let text = match self.cfg.storage_limit_gb {
                Some(gb) => format!("Using {used} of {gb} GB ({files})"),
                None => format!("Using {used} ({files})"),
            };
            ui.label(egui::RichText::new(text).weak());
            if let Some(limit) = self.cfg.storage_limit_bytes() {
                ui.add(egui::ProgressBar::new((u.bytes as f32 / limit as f32).min(1.0)).desired_height(6.0));
                if u.bytes > limit && self.cfg != self.saved {
                    ui.label(
                        egui::RichText::new("Saving will delete the oldest recordings now to get under the limit.")
                            .small()
                            .color(RED),
                    );
                }
            }
        }
    }

    fn video(&mut self, ui: &mut egui::Ui) {
        ui.heading("Video (NVIDIA NVENC)");
        let v = &mut self.cfg.video;
        egui::Grid::new("video").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
            ui.label("Resolution");
            egui::ComboBox::from_id_salt("res").selected_text(v.resolution.label()).show_ui(ui, |ui| {
                for r in Resolution::ALL {
                    ui.selectable_value(&mut v.resolution, r, r.label());
                }
            });
            ui.end_row();

            ui.label("Frame rate");
            egui::ComboBox::from_id_salt("fps").selected_text(format!("{} fps", v.fps)).show_ui(ui, |ui| {
                for f in FPS_CHOICES {
                    ui.selectable_value(&mut v.fps, f, format!("{f} fps"));
                }
            });
            ui.end_row();

            ui.label("Codec");
            egui::ComboBox::from_id_salt("codec").selected_text(v.codec.label()).show_ui(ui, |ui| {
                for c in Codec::ALL {
                    ui.selectable_value(&mut v.codec, c, c.label());
                }
            });
            ui.end_row();

            ui.label("Quality");
            let mut level = v.quality as usize;
            let slider = egui::Slider::new(&mut level, 0..=Quality::ALL.len() - 1)
                .show_value(false)
                .text(v.quality.label());
            if ui.add(slider).changed() {
                v.quality = Quality::ALL[level];
            }
            ui.end_row();

            ui.label("Bitrate cap");
            ui.horizontal(|ui| {
                let mut auto = v.bitrate_cap_mbps.is_none();
                if ui.checkbox(&mut auto, "Automatic").changed() {
                    v.bitrate_cap_mbps = if auto { None } else { Some(40) };
                }
                if let Some(cap) = &mut v.bitrate_cap_mbps {
                    ui.add(egui::Slider::new(cap, 4..=300).suffix(" Mbps").logarithmic(true));
                }
            });
            ui.end_row();
        });

        ui.add_enabled_ui(v.codec.supports_full_chroma(), |ui| {
            ui.checkbox(&mut v.full_chroma, "Full colour detail (4:4:4) — much sharper coloured text and UI")
                .on_disabled_hover_text("NVENC can't encode AV1 in 4:4:4. Use H.264 or HEVC.");
        });
        if v.full_chroma && v.codec.supports_full_chroma() {
            ui.label(
                egui::RichText::new(
                    "Plays in VLC/mpv and uploads fine to YouTube, but Windows' Media Player, browsers and \
                     Discord previews can't play 4:4:4. For sharp text that plays everywhere, use 2× \
                     pixel-perfect resolution instead.",
                )
                .small()
                .color(AMBER),
            );
        }
        let hint = match v.resolution {
            Resolution::Native => None,
            Resolution::Double => Some(
                "Every game pixel becomes a crisp 2×2 block: full colour detail in any player, and \
                 YouTube serves it at 4K bitrates. Files are ~2.5× bigger.",
            ),
            _ => Some(
                "Choosing a size bigger than your game window upscales it: no extra detail locally \
                 (YouTube does give higher resolutions more bitrate). 2× pixel-perfect is sharper.",
            ),
        };
        if let Some(hint) = hint {
            ui.label(egui::RichText::new(hint).small().weak());
        }
    }

    fn audio(&mut self, ui: &mut egui::Ui) {
        ui.heading("Audio");
        let a = &mut self.cfg.audio;
        ui.checkbox(&mut a.game_audio, "Record computer audio");
        ui.add_enabled_ui(a.game_audio, |ui| {
            ui.indent("source", |ui| {
                for kind in AudioSourceKind::ALL {
                    ui.radio_value(&mut a.source, kind, kind.label());
                }
            });
        });
        ui.checkbox(&mut a.mic, "Record microphone");
        ui.add_enabled_ui(a.mic, |ui| {
            ui.indent("mic", |ui| {
                let current = a.mic_device.clone().unwrap_or_else(|| "Windows default".into());
                egui::ComboBox::from_label("Microphone").selected_text(current).show_ui(ui, |ui| {
                    ui.selectable_value(&mut a.mic_device, None, "Windows default");
                    for m in &self.mics {
                        ui.selectable_value(&mut a.mic_device, Some(m.clone()), m);
                    }
                });
            });
        });

        let both = a.game_audio && a.mic;
        ui.add_enabled_ui(both, |ui| {
            ui.checkbox(&mut a.mixed_track, "Add a mixed track first (recommended)").on_hover_text(
                "Most players and YouTube only play the first audio track. With this on, track 1 is \
                 game + mic mixed, and the separate tracks follow for editing.",
            );
            ui.add_enabled_ui(a.mixed_track, |ui| {
                egui::Grid::new("mix").num_columns(2).show(ui, |ui| {
                    ui.label(format!("{} volume in mix", a.source.track_title()));
                    ui.add(egui::Slider::new(&mut a.game_volume, 0..=MAX_VOLUME_PERCENT).suffix("%"));
                    ui.end_row();
                    ui.label("Mic volume in mix");
                    ui.add(egui::Slider::new(&mut a.mic_volume, 0..=MAX_VOLUME_PERCENT).suffix("%"));
                    ui.end_row();
                });
            });
        });
        let tracks = match (a.game_audio, a.mic, a.mixed_track) {
            (true, true, true) => format!("Tracks: 1 Mixed · 2 {} · 3 Mic", a.source.track_title()),
            (true, true, false) => format!("Tracks: 1 {} · 2 Mic", a.source.track_title()),
            (true, false, _) => format!("Tracks: 1 {}", a.source.track_title()),
            (false, true, _) => "Tracks: 1 Mic".to_owned(),
            (false, false, _) => "No audio".to_owned(),
        };
        ui.label(egui::RichText::new(tracks).weak());
        ui.add(egui::Slider::new(&mut a.bitrate_kbps, 96..=320).text("AAC bitrate (kbps)"));
    }

    fn games(&mut self, ui: &mut egui::Ui) {
        ui.heading("Games");
        ui.label("Recording starts automatically while an enabled game is running. These switches apply immediately.");
        ui.add_space(4.0);

        let mut toggled = None;
        for game in self.cfg.games.iter_mut().filter(|g| g.is_builtin()) {
            ui.horizontal(|ui| {
                if ui.checkbox(&mut game.enabled, egui::RichText::new(&game.name).strong()).changed() {
                    toggled = Some((game.id.clone(), game.enabled));
                }
                if let Some(note) = profile::builtin_note(&game.id) {
                    ui.label(egui::RichText::new(note).small().weak());
                }
            });
        }

        ui.add_space(6.0);
        let customs = self.cfg.games.iter().filter(|g| !g.is_builtin()).count();
        egui::CollapsingHeader::new(format!("Custom games ({customs})"))
            .default_open(customs > 0)
            .show(ui, |ui| {
                ui.label(
                    egui::RichText::new(
                        "Detected when the window title matches the regex and belongs to one of the \
                         executables. Click Save to apply changes here.",
                    )
                    .small()
                    .weak(),
                );
                let mut remove = None;
                for (i, (game, exe)) in self.cfg.games.iter_mut().zip(self.exe_text.iter_mut()).enumerate() {
                    if game.is_builtin() {
                        continue;
                    }
                    egui::Frame::group(ui.style()).show(ui, |ui| {
                        ui.horizontal(|ui| {
                            if ui.checkbox(&mut game.enabled, "").changed() {
                                toggled = Some((game.id.clone(), game.enabled));
                            }
                            ui.text_edit_singleline(&mut game.name);
                            if ui.small_button("Remove").clicked() {
                                remove = Some(i);
                            }
                        });
                        egui::Grid::new(("game", i)).num_columns(2).show(ui, |ui| {
                            ui.label("Executables");
                            ui.text_edit_singleline(exe);
                            ui.end_row();
                            ui.label("Title regex");
                            ui.text_edit_singleline(&mut game.window_title_regex);
                            ui.end_row();
                        });
                    });
                }
                if let Some(i) = remove {
                    self.cfg.games.remove(i);
                    self.exe_text.remove(i);
                }
                if ui.button("Add custom game").clicked() {
                    let used: Vec<String> = self.cfg.games.iter().map(|g| g.id.clone()).collect();
                    self.cfg.games.push(GameProfile {
                        id: profile::unique_custom_id("New game", &used),
                        name: "New game".into(),
                        exe_names: vec![],
                        window_title_regex: ".*".into(),
                        enabled: true,
                    });
                    self.exe_text.push(String::new());
                }
            });

        if let Some((id, enabled)) = toggled {
            self.apply_game_toggle(&id, enabled);
        }
    }

    /// Writes one game's on/off switch straight to disk (the tray applies it
    /// at once), without saving any other unsaved edits in the form.
    fn apply_game_toggle(&mut self, id: &str, enabled: bool) {
        let path = paths::config_file();
        let result = Config::load_or_create(&path).and_then(|mut disk| {
            if disk.set_game_enabled(id, enabled) {
                disk.save(&path)?;
            }
            Ok(())
        });
        match result {
            Ok(()) => {
                self.saved.set_game_enabled(id, enabled);
                self.config_mtime = config_mtime();
            }
            Err(e) => self.message = Some((format!("Couldn't update game: {e:#}"), false)),
        }
    }

    /// Picks up changes made elsewhere (e.g. game toggles in the tray menu).
    fn reload_if_changed_on_disk(&mut self) {
        let mtime = config_mtime();
        if mtime == self.config_mtime {
            return;
        }
        self.config_mtime = mtime;
        let Ok(disk) = Config::load_or_create(&paths::config_file()) else { return };
        let dirty = self.cfg != self.saved;
        for g in &disk.games {
            self.cfg.set_game_enabled(&g.id, g.enabled);
        }
        if dirty {
            // Keep the user's unsaved edits; only the toggles were merged in.
            self.saved = disk;
        } else {
            self.saved = disk.clone();
            self.cfg = disk;
            self.exe_text = self.cfg.games.iter().map(|g| g.exe_names.join(", ")).collect();
        }
    }
}

fn config_mtime() -> Option<std::time::SystemTime> {
    std::fs::metadata(paths::config_file()).and_then(|m| m.modified()).ok()
}

/// A coloured status dot; `pulse` makes it breathe like a REC light.
fn status_dot(ui: &mut egui::Ui, color: egui::Color32, pulse: bool) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(18.0, 18.0), egui::Sense::hover());
    let color = if pulse {
        let t = ui.input(|i| i.time);
        let a = 0.55 + 0.45 * (t * std::f64::consts::TAU / 1.6).sin().abs();
        color.gamma_multiply(a as f32)
    } else {
        color
    };
    ui.painter().circle_filled(rect.center(), 7.0, color);
}

/// Space used by saved recordings, as last measured.
struct Usage {
    dir: std::path::PathBuf,
    bytes: u64,
    files: usize,
    measured: std::time::Instant,
}
