//! Background thread that owns the recording state machine:
//! wait for a game → start the encoder → keep the buffer trimmed → export
//! clips/sessions on request → tear down when the game closes.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use chrono::Local;
use serde::{Deserialize, Serialize};
use crossbeam_channel::{Receiver, RecvTimeoutError};
use ptv_capture::detect::{self, GameWindow};
use ptv_core::clip::{self, ExportKind};
use ptv_core::config::Config;
use ptv_core::profile::{safe_name, ProfileMatcher};
use ptv_core::ring::{self, Segment};
use ptv_core::storage;

use crate::export;
use crate::paths;
use crate::recorder::Recorder;

const SCAN_INTERVAL: Duration = Duration::from_secs(2);
const META_FILE: &str = "meta.txt";
const SESSION_FILE: &str = "session.txt";

#[derive(Debug)]
pub enum Command {
    SaveClip,
    ToggleSession,
    Reload(Box<Config>),
    Shutdown,
}

/// Times are Unix seconds so they mean the same thing in the settings
/// window's process.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Status {
    Waiting,
    Recording {
        game: String,
        /// When the current encoder started filling the replay buffer.
        since: u64,
        /// When the current full-session file started, if one is recording.
        session_since: Option<u64>,
        /// What's being encoded, e.g. "1900×1030 · 60 fps · H.264 · High quality".
        #[serde(default)]
        video: String,
    },
    /// Game found but the encoder keeps failing.
    Failing { game: String, error: String },
}

impl Status {
    pub fn session_since(&self) -> Option<u64> {
        match self {
            Status::Recording { session_since, .. } => *session_since,
            _ => None,
        }
    }

    pub fn is_recording(&self) -> bool {
        matches!(self, Status::Recording { .. })
    }
}

pub fn unix_now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

#[derive(Debug, Clone)]
pub enum Event {
    Status(Status),
    Saved { path: PathBuf, kind: ExportKind },
    /// Old recordings were deleted to stay under the storage limit.
    Cleaned { count: usize, bytes: u64 },
    Error(String),
}

pub type EventSink = Arc<dyn Fn(Event) + Send + Sync>;

struct Active {
    window: GameWindow,
    dir: PathBuf,
    /// Held open exclusively for the buffer's lifetime: see [`buffer_in_use`].
    _lock: std::fs::File,
    recorder: Option<Recorder>,
    audio_titles: Vec<String>,
    /// First segment of an in-progress full-session recording.
    session_from: Option<u64>,
    session_since: Option<u64>,
    recording_since: u64,
    /// First segment written by the current encoder. Segments from earlier
    /// encoders may have a different resolution/codec, so a single .mp4
    /// never mixes the two.
    epoch_from: u64,
    failures: u32,
    retry_at: Instant,
    last_error: Option<String>,
}

pub struct Supervisor {
    cfg: Config,
    matchers: Vec<ProfileMatcher>,
    ffmpeg: PathBuf,
    sink: EventSink,
    active: Option<Active>,
    exports_in_flight: Arc<AtomicUsize>,
    last_scan: Option<Instant>,
    last_status: Option<Status>,
}

impl Supervisor {
    pub fn new(cfg: Config, ffmpeg: PathBuf, sink: EventSink) -> Self {
        Self {
            matchers: matchers(&cfg),
            cfg,
            ffmpeg,
            sink,
            active: None,
            exports_in_flight: Arc::new(AtomicUsize::new(0)),
            last_scan: None,
            last_status: None,
        }
    }

    pub fn run(mut self, commands: Receiver<Command>) {
        self.recover_leftovers();
        self.enforce_storage_soon();
        loop {
            match commands.recv_timeout(Duration::from_millis(500)) {
                Ok(Command::Shutdown) | Err(RecvTimeoutError::Disconnected) => break,
                Ok(cmd) => self.handle(cmd),
                Err(RecvTimeoutError::Timeout) => {}
            }
            self.tick();
        }
        self.stop_game();
        self.wait_for_exports();
    }

    fn handle(&mut self, cmd: Command) {
        match cmd {
            Command::SaveClip => self.save_clip(),
            Command::ToggleSession => self.toggle_session(),
            Command::Reload(cfg) => self.reload(*cfg),
            Command::Shutdown => unreachable!("handled in run"),
        }
    }

    fn tick(&mut self) {
        match &mut self.active {
            None => {
                if self.last_scan.is_none_or(|t| t.elapsed() >= SCAN_INTERVAL) {
                    self.last_scan = Some(Instant::now());
                    if let Some(window) = detect::find_game(&self.matchers) {
                        self.start_game(window);
                    }
                }
            }
            Some(active) if !detect::window_alive(active.window.hwnd) => {
                tracing::info!("{} closed", active.window.profile.name);
                self.stop_game();
            }
            Some(_) => {
                let mut active = self.active.take().expect("matched Some");
                if let Some(rec) = &mut active.recorder {
                    if let Some(err) = rec.exited() {
                        tracing::error!("{err}");
                        // A long healthy run resets the backoff.
                        if rec.uptime() > Duration::from_secs(60) {
                            active.failures = 0;
                        }
                        active.failures += 1;
                        let backoff = [2, 5, 15, 30][(active.failures as usize - 1).min(3)];
                        active.retry_at = Instant::now() + Duration::from_secs(backoff);
                        active.last_error = Some(err);
                        active.recorder.take().unwrap().stop();
                    }
                }
                if active.recorder.is_none() && Instant::now() >= active.retry_at {
                    self.restart_recorder(&mut active);
                }
                if self.exports_in_flight.load(Ordering::SeqCst) == 0 {
                    prune(&active.dir, self.cfg.buffer_seconds, active.session_from);
                }
                self.active = Some(active);
            }
        }
        self.publish_status();
    }

    /// Starts a fresh encoder for an already-running game. A full session in
    /// progress is saved and continued in a new file, since the new encoder's
    /// output may not be concatenable with the old one's.
    fn restart_recorder(&mut self, active: &mut Active) {
        if let Some(rec) = active.recorder.take() {
            rec.stop();
        }
        let has_session_footage = active.session_from.is_some_and(|from| {
            ring::scan(&active.dir).unwrap_or_default().iter().any(|s| s.index >= from)
        });
        let continue_session = active.session_from.is_some();
        if has_session_footage {
            self.finish_session(active, false);
        }
        start_recorder(&self.ffmpeg, &self.cfg, active);
        if continue_session {
            begin_session(active);
        }
    }

    fn start_game(&mut self, window: GameWindow) {
        tracing::info!("detected {} (pid {}, \"{}\")", window.profile.name, window.pid, window.title);
        let dir = paths::buffer_root().join(format!(
            "{}_{}",
            safe_name(&window.profile.name),
            Local::now().format("%Y%m%d-%H%M%S")
        ));
        let lock = std::fs::create_dir_all(&dir).and_then(|_| lock_buffer(&dir));
        let lock = match lock {
            Ok(l) => l,
            Err(e) => {
                self.emit(Event::Error(format!("Can't create buffer folder: {e}")));
                return;
            }
        };
        let mut active = Active {
            window,
            dir,
            _lock: lock,
            recorder: None,
            audio_titles: Vec::new(),
            session_from: None,
            session_since: None,
            recording_since: unix_now(),
            epoch_from: 0,
            failures: 0,
            retry_at: Instant::now(),
            last_error: None,
        };
        start_recorder(&self.ffmpeg, &self.cfg, &mut active);
        if self.cfg.record_full_session {
            begin_session(&mut active);
        }
        self.active = Some(active);
    }

    fn stop_game(&mut self) {
        let Some(mut active) = self.active.take() else { return };
        if let Some(rec) = active.recorder.take() {
            rec.stop();
        }
        let dir = active.dir.clone();
        if active.session_from.is_some() {
            // Exports the session, then deletes the buffer once idle.
            self.finish_session(&mut active, true);
        } else {
            delete_when_idle(dir, self.exports_in_flight.clone());
        }
    }

    fn reload(&mut self, cfg: Config) {
        let restart = cfg.capture_changed(&self.cfg);
        let session_turned_on = cfg.record_full_session && !self.cfg.record_full_session;
        let storage_changed = cfg.storage_limit_gb != self.cfg.storage_limit_gb || cfg.output_dir != self.cfg.output_dir;
        self.cfg = cfg;
        self.matchers = matchers(&self.cfg);
        // Rescan now rather than in up to 2 s: a game may have just been enabled.
        self.last_scan = None;
        if storage_changed {
            self.enforce_storage_soon();
        }
        let Some(mut active) = self.active.take() else { return };
        if !self.cfg.game_enabled(&active.window.profile.id) {
            tracing::info!("{} capture turned off, stopping", active.window.profile.name);
            self.active = Some(active);
            self.stop_game(); // saves a full session if one was running
            return;
        }
        if restart {
            tracing::info!("capture settings changed, restarting encoder");
            active.failures = 0;
            self.restart_recorder(&mut active);
        }
        if session_turned_on && active.session_from.is_none() {
            begin_session(&mut active);
        }
        self.active = Some(active);
    }

    fn save_clip(&mut self) {
        let Some(active) = &self.active else {
            self.emit(Event::Error("No game running — nothing to clip.".into()));
            return;
        };
        let latest = match ring::scan(&active.dir) {
            Ok(s) => s.last().map(|s| s.index),
            Err(e) => return self.emit(Event::Error(format!("Reading buffer failed: {e}"))),
        };
        let Some(latest) = latest else {
            return self.emit(Event::Error("Nothing recorded yet — try again in a moment.".into()));
        };
        let out = clip::output_path(&self.cfg.output_dir, &active.window.profile.name, ExportKind::Clip, Local::now());
        let titles = active.audio_titles.clone();
        let source = ExportSource::ClipUpTo {
            dir: active.dir.clone(),
            first: active.epoch_from,
            last: latest,
            seconds: self.cfg.buffer_seconds,
        };
        self.spawn_export(source, out, titles, ExportKind::Clip, None);
    }

    fn toggle_session(&mut self) {
        let Some(mut active) = self.active.take() else {
            self.emit(Event::Error("No game running — session recording starts once a game is detected.".into()));
            return;
        };
        if active.session_from.is_some() {
            self.finish_session(&mut active, false);
        } else {
            begin_session(&mut active);
        }
        self.active = Some(active);
        self.publish_status();
    }

    /// Exports everything since the session started. With `cleanup`, the
    /// buffer folder is removed afterwards (the game has closed).
    fn finish_session(&mut self, active: &mut Active, cleanup: bool) {
        let Some(from) = active.session_from.take() else { return };
        active.session_since = None;
        let _ = std::fs::remove_file(active.dir.join(SESSION_FILE));
        let segs: Vec<Segment> = ring::scan(&active.dir)
            .unwrap_or_default()
            .into_iter()
            .filter(|s| s.index >= from)
            .collect();
        let out = clip::output_path(&self.cfg.output_dir, &active.window.profile.name, ExportKind::Session, Local::now());
        let cleanup = cleanup.then(|| active.dir.clone());
        self.spawn_export(ExportSource::Segments(segs), out, active.audio_titles.clone(), ExportKind::Session, cleanup);
    }

    fn storage_policy(&self) -> StoragePolicy {
        StoragePolicy { output_dir: self.cfg.output_dir.clone(), limit: self.cfg.storage_limit_bytes() }
    }

    /// Applies the storage limit now, unless a save is in progress (that
    /// save will apply it when it finishes).
    fn enforce_storage_soon(&self) {
        if self.exports_in_flight.load(Ordering::SeqCst) > 0 {
            return;
        }
        let storage = self.storage_policy();
        if storage.limit.is_none() {
            return;
        }
        let sink = self.sink.clone();
        std::thread::spawn(move || enforce_storage_limit(&storage, &sink));
    }

    fn spawn_export(
        &self,
        source: ExportSource,
        out: PathBuf,
        titles: Vec<String>,
        kind: ExportKind,
        cleanup: Option<PathBuf>,
    ) {
        let ffmpeg = self.ffmpeg.clone();
        let in_flight = self.exports_in_flight.clone();
        let sink = self.sink.clone();
        let storage = self.storage_policy();
        // Counted from now so pruning pauses while we wait for the segment.
        in_flight.fetch_add(1, Ordering::SeqCst);
        std::thread::spawn(move || {
            let started = Instant::now();
            let segs = source.resolve();
            let result = export::export(&ffmpeg, &segs, out, &titles);
            let still_exporting = in_flight.fetch_sub(1, Ordering::SeqCst) > 1;
            match result {
                Ok(path) => {
                    tracing::info!("saved {} in {:?}", path.display(), started.elapsed());
                    sink(Event::Saved { path, kind });
                }
                Err(e) => {
                    tracing::error!("export failed: {e:#}");
                    sink(Event::Error(format!("Saving failed: {e:#}")));
                }
            }
            // The last export to finish enforces the limit, so a file that's
            // still being written is never a deletion candidate.
            if !still_exporting {
                enforce_storage_limit(&storage, &sink);
            }
            if let Some(dir) = cleanup {
                delete_when_idle(dir, in_flight);
            }
        });
    }

    /// Buffers left behind by a crash. Sessions in progress are salvaged.
    fn recover_leftovers(&mut self) {
        let Ok(entries) = std::fs::read_dir(paths::buffer_root()) else { return };
        for dir in entries.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.is_dir()) {
            if buffer_in_use(&dir) {
                tracing::warn!("{} is in use by another playerstv process, leaving it alone", dir.display());
                continue;
            }
            let session = std::fs::read_to_string(dir.join(SESSION_FILE))
                .ok()
                .and_then(|s| s.trim().parse::<u64>().ok());
            let Some(from) = session else {
                let _ = std::fs::remove_dir_all(&dir);
                continue;
            };
            let meta = std::fs::read_to_string(dir.join(META_FILE)).unwrap_or_default();
            let mut lines = meta.lines();
            let game = lines.next().unwrap_or("Game").to_owned();
            let titles: Vec<String> = lines.next().unwrap_or("").split(',').filter(|s| !s.is_empty()).map(String::from).collect();
            let segs: Vec<Segment> = ring::scan(&dir).unwrap_or_default().into_iter().filter(|s| s.index >= from).collect();
            tracing::info!("recovering unsaved {game} session from {}", dir.display());
            let out = clip::output_path(&self.cfg.output_dir, &game, ExportKind::Session, Local::now());
            self.spawn_export(ExportSource::Segments(segs), out, titles, ExportKind::Session, Some(dir));
        }
    }

    fn wait_for_exports(&self) {
        let deadline = Instant::now() + Duration::from_secs(120);
        while self.exports_in_flight.load(Ordering::SeqCst) > 0 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    fn status(&self) -> Status {
        match &self.active {
            None => Status::Waiting,
            Some(a) => match (&a.recorder, &a.last_error) {
                (None, Some(err)) => Status::Failing { game: a.window.profile.name.clone(), error: err.clone() },
                _ => Status::Recording {
                    game: a.window.profile.name.clone(),
                    since: a.recording_since,
                    session_since: a.session_since,
                    video: a.recorder.as_ref().map(|r| r.video_summary.clone()).unwrap_or_default(),
                },
            },
        }
    }

    fn publish_status(&mut self) {
        let status = self.status();
        if self.last_status.as_ref() != Some(&status) {
            self.last_status = Some(status.clone());
            self.emit(Event::Status(status));
        }
    }

    fn emit(&self, event: Event) {
        (self.sink)(event);
    }
}

enum ExportSource {
    Segments(Vec<Segment>),
    /// The newest `seconds` of footage ending with segment `last`. That
    /// segment was still being written when the hotkey was pressed, so we
    /// wait for ffmpeg to close it: otherwise the last moments (the bit the
    /// player actually wanted) would still be in ffmpeg's buffers. Segments
    /// before `first` belong to an earlier encoder and are left out.
    ClipUpTo { dir: PathBuf, first: u64, last: u64, seconds: u32 },
}

impl ExportSource {
    fn resolve(self) -> Vec<Segment> {
        match self {
            ExportSource::Segments(s) => s,
            ExportSource::ClipUpTo { dir, first, last, seconds } => {
                let deadline = Instant::now() + Duration::from_secs(ring::SEGMENT_SECONDS as u64 + 2);
                while !ring::segment_exists(&dir, last + 1) && Instant::now() < deadline {
                    std::thread::sleep(Duration::from_millis(100));
                }
                let segs: Vec<Segment> = ring::scan(&dir)
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|s| (first..=last).contains(&s.index))
                    .collect();
                ring::select_for_clip(&segs, seconds).to_vec()
            }
        }
    }
}

fn matchers(cfg: &Config) -> Vec<ProfileMatcher> {
    cfg.games
        .iter()
        .filter_map(|g| {
            let m = g.matcher();
            if m.is_none() {
                tracing::warn!("invalid window title regex for {}: {}", g.name, g.window_title_regex);
            }
            m
        })
        .collect()
}

fn start_recorder(ffmpeg: &Path, cfg: &Config, active: &mut Active) {
    let start = ring::next_index(&ring::scan(&active.dir).unwrap_or_default());
    active.epoch_from = start;
    match Recorder::start(ffmpeg, &active.window, cfg, &active.dir, start) {
        Ok(rec) => {
            active.audio_titles = rec.audio_titles.clone();
            let meta = format!("{}\n{}\n", active.window.profile.name, active.audio_titles.join(","));
            let _ = std::fs::write(active.dir.join(META_FILE), meta);
            active.recorder = Some(rec);
            active.recording_since = unix_now();
            active.last_error = None;
        }
        Err(e) => {
            tracing::error!("failed to start recorder: {e:#}");
            active.failures += 1;
            active.retry_at = Instant::now() + Duration::from_secs(10);
            active.last_error = Some(format!("{e:#}"));
        }
    }
}

fn begin_session(active: &mut Active) {
    // Include the segment being written right now, but nothing from an
    // earlier encoder (a just-restarted one may not have written a file yet).
    let from = ring::scan(&active.dir)
        .ok()
        .and_then(|s| s.last().map(|s| s.index))
        .unwrap_or(0)
        .max(active.epoch_from);
    tracing::info!("full-session recording started at segment {from}");
    let _ = std::fs::write(active.dir.join(SESSION_FILE), from.to_string());
    active.session_from = Some(from);
    active.session_since = Some(unix_now());
}

/// Where recordings are saved and how big they may get in total.
struct StoragePolicy {
    output_dir: PathBuf,
    limit: Option<u64>,
}

/// Deletes the oldest saved recordings until they fit under the limit.
fn enforce_storage_limit(policy: &StoragePolicy, sink: &EventSink) {
    // Saves finishing together could both get here; one pass at a time.
    static RUNNING: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let Some(limit) = policy.limit else { return };
    let _guard = RUNNING.lock().unwrap_or_else(|e| e.into_inner());

    let recs = storage::scan(&policy.output_dir);
    let plan = storage::plan_cleanup(&recs, limit);
    let (mut count, mut bytes) = (0, 0);
    for r in &plan {
        match std::fs::remove_file(&r.path) {
            Ok(()) => {
                tracing::info!("storage limit: deleted {} ({})", r.path.display(), storage::format_size(r.size));
                count += 1;
                bytes += r.size;
            }
            // Open in a player, say: skip it, try again after the next save.
            Err(e) => tracing::warn!("storage limit: couldn't delete {}: {e}", r.path.display()),
        }
    }
    if count > 0 {
        sink(Event::Cleaned { count, bytes });
    }
}

const LOCK_FILE: &str = "in-use.lock";

/// Opens the buffer's lock file with no sharing allowed, so any other
/// process trying to open (or delete) it gets a sharing violation.
fn lock_buffer(dir: &Path) -> std::io::Result<std::fs::File> {
    use std::os::windows::fs::OpenOptionsExt;
    std::fs::OpenOptions::new().write(true).create(true).truncate(true).share_mode(0).open(dir.join(LOCK_FILE))
}

/// True while a running playerstv owns this buffer. Without this check a
/// second process (a `--smoke` run, a dev build, a future second launch
/// path) would treat a live buffer as a crash leftover and delete it.
fn buffer_in_use(dir: &Path) -> bool {
    let lock = dir.join(LOCK_FILE);
    lock.exists() && lock_buffer(dir).is_err()
}

fn prune(dir: &Path, buffer_seconds: u32, keep_from: Option<u64>) {
    let Ok(segs) = ring::scan(dir) else { return };
    for seg in ring::prunable(&segs, buffer_seconds, keep_from) {
        // The segment ffmpeg holds open can't be deleted; we'll retry next tick.
        let _ = std::fs::remove_file(&seg.path);
    }
}

fn delete_when_idle(dir: PathBuf, in_flight: Arc<AtomicUsize>) {
    std::thread::spawn(move || {
        while in_flight.load(Ordering::SeqCst) > 0 {
            std::thread::sleep(Duration::from_millis(200));
        }
        // Our lock file / ffmpeg's last segment may be closed a moment later.
        for attempt in 1..=10 {
            match std::fs::remove_dir_all(&dir) {
                Ok(()) => return,
                Err(_) if attempt < 10 => std::thread::sleep(Duration::from_millis(300)),
                Err(e) => tracing::warn!("couldn't remove {}: {e}", dir.display()),
            }
        }
    });
}
