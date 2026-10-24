//! Background thread that owns the recording state machine:
//! wait for a game → start the encoder → keep the buffer trimmed → export
//! clips/sessions on request → tear down when the game closes.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::Local;
use crossbeam_channel::{Receiver, RecvTimeoutError};
use ptv_capture::detect::{self, GameWindow};
use ptv_core::clip::{self, ExportKind};
use ptv_core::config::{Config, VideoConfig};
use ptv_core::ffmpeg_args::{self, CaptureTarget};
use ptv_core::profile::{safe_name, ProfileMatcher};
use ptv_core::ring::{self, Segment};
use ptv_core::storage;

use crate::export;
use crate::paths;
use crate::recorder::Recorder;

const SCAN_INTERVAL: Duration = Duration::from_secs(2);
/// How long a new window size must hold before the encoder restarts for it,
/// so dragging a window edge doesn't restart it over and over.
const RESIZE_SETTLE: Duration = Duration::from_secs(2);
const META_FILE: &str = "meta.txt";
const SESSION_FILE: &str = "session.txt";

#[derive(Debug)]
pub enum Command {
    SaveClip,
    ToggleSession,
    Reload(Box<Config>),
    Shutdown,
}

pub use ptv_common::status::{unix_now, Status};

#[derive(Debug, Clone)]
pub enum Event {
    Status(Status),
    Saved {
        path: PathBuf,
        kind: ExportKind,
    },
    /// Old recordings were deleted to stay under the storage limit.
    Cleaned {
        count: usize,
        bytes: u64,
    },
    Error(String),
}

pub type EventSink = Arc<dyn Fn(Event) + Send + Sync>;

struct Active {
    window: GameWindow,
    /// The settings this game records with: [`Config::for_game`].
    settings: Config,
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
    /// A window size that differs from the one being encoded, and when it
    /// was first seen.
    pending_resize: Option<((u32, u32), Instant)>,
}

pub struct Supervisor {
    cfg: Config,
    matchers: Vec<ProfileMatcher>,
    ffmpeg: PathBuf,
    sink: EventSink,
    active: Option<Active>,
    exports_in_flight: Arc<AtomicUsize>,
    storage: SharedStoragePolicy,
    last_scan: Option<Instant>,
    last_status: Option<Status>,
}

impl Supervisor {
    pub fn new(cfg: Config, ffmpeg: PathBuf, sink: EventSink) -> Self {
        let storage = Arc::new(std::sync::Mutex::new(StoragePolicy::of(&cfg)));
        Self {
            storage,
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
                } else if window_resized(&mut active) {
                    tracing::info!("{} window resized, restarting encoder", active.window.profile.name);
                    active.failures = 0;
                    self.restart_recorder(&mut active);
                }
                if self.exports_in_flight.load(Ordering::SeqCst) == 0 {
                    prune(&active.dir, active.settings.buffer_seconds, active.session_from);
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
        let has_session_footage = active
            .session_from
            .is_some_and(|from| ring::scan(&active.dir).unwrap_or_default().iter().any(|s| s.index >= from));
        let continue_session = active.session_from.is_some();
        if has_session_footage {
            self.finish_session(active, false);
        }
        start_recorder(&self.ffmpeg, active);
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
            settings: self.cfg.for_game(&window.profile.id),
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
            pending_resize: None,
        };
        start_recorder(&self.ffmpeg, &mut active);
        if active.settings.record_full_session {
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
        let policy = StoragePolicy::of(&cfg);
        let storage_changed = {
            let mut current = self.storage.lock().unwrap_or_else(|e| e.into_inner());
            let changed = *current != policy;
            *current = policy;
            changed
        };
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
        // Only this game's settings matter: another game's overrides
        // changing mustn't restart (and split) this recording.
        let settings = self.cfg.for_game(&active.window.profile.id);
        let restart = settings.capture_changed(&active.settings);
        let session_turned_on = settings.record_full_session && !active.settings.record_full_session;
        active.settings = settings;
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
            self.emit(Event::Error("No game running, so there's nothing to clip.".into()));
            return;
        };
        let latest = match ring::scan(&active.dir) {
            Ok(s) => s.last().map(|s| s.index),
            Err(e) => return self.emit(Event::Error(format!("Reading buffer failed: {e}"))),
        };
        let Some(latest) = latest else {
            return self.emit(Event::Error("Nothing recorded yet. Try again in a few seconds.".into()));
        };
        let out = clip::output_path(&self.cfg.output_dir, &active.window.profile.name, ExportKind::Clip, Local::now());
        let titles = active.audio_titles.clone();
        let source = ExportSource::ClipUpTo {
            dir: active.dir.clone(),
            first: active.epoch_from,
            last: latest,
            seconds: active.settings.buffer_seconds,
        };
        self.spawn_export(source, out, titles, ExportKind::Clip, None);
    }

    fn toggle_session(&mut self) {
        let Some(mut active) = self.active.take() else {
            self.emit(Event::Error("No game running. Session recording starts when playerstv detects a game.".into()));
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
        let segs: Vec<Segment> =
            ring::scan(&active.dir).unwrap_or_default().into_iter().filter(|s| s.index >= from).collect();
        let out =
            clip::output_path(&self.cfg.output_dir, &active.window.profile.name, ExportKind::Session, Local::now());
        let cleanup = cleanup.then(|| active.dir.clone());
        self.spawn_export(ExportSource::Segments(segs), out, active.audio_titles.clone(), ExportKind::Session, cleanup);
    }

    /// Applies the storage limit now, unless a save is in progress: that
    /// save applies it when it finishes, reading the *current* policy then.
    fn enforce_storage_soon(&self) {
        if self.exports_in_flight.load(Ordering::SeqCst) > 0 {
            return;
        }
        let storage = self.storage.clone();
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
        let storage = self.storage.clone();
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
            let session =
                std::fs::read_to_string(dir.join(SESSION_FILE)).ok().and_then(|s| s.trim().parse::<u64>().ok());
            let Some(from) = session else {
                let _ = std::fs::remove_dir_all(&dir);
                continue;
            };
            let meta = std::fs::read_to_string(dir.join(META_FILE)).unwrap_or_default();
            let mut lines = meta.lines();
            let game = lines.next().unwrap_or("Game").to_owned();
            let titles: Vec<String> =
                lines.next().unwrap_or("").split(',').filter(|s| !s.is_empty()).map(String::from).collect();
            let segs: Vec<Segment> =
                ring::scan(&dir).unwrap_or_default().into_iter().filter(|s| s.index >= from).collect();
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
                    buffer_seconds: a.settings.buffer_seconds,
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
    ClipUpTo {
        dir: PathBuf,
        first: u64,
        last: u64,
        seconds: u32,
    },
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

/// True once the window has settled at a size that would encode to a
/// different frame size than the running encoder's. gfxcapture fixes its
/// output size at start, so without a restart a game that changes size after
/// launch (RuneLite opening its sidebar, say) ends up letterboxed.
fn window_resized(active: &mut Active) -> bool {
    let Some(rec) = &active.recorder else {
        active.pending_resize = None;
        return false;
    };
    let size = detect::client_size(active.window.hwnd);
    resize_settled(&mut active.pending_resize, rec.target, size, &active.settings.video, Instant::now())
}

/// [`window_resized`]'s decision for a window now `size` big, while
/// `encoding` is what the encoder was started with. `pending` carries a
/// changed size between calls.
fn resize_settled(
    pending: &mut Option<((u32, u32), Instant)>,
    encoding: CaptureTarget,
    size: (u32, u32),
    video: &VideoConfig,
    now: Instant,
) -> bool {
    let target = CaptureTarget { width: size.0, height: size.1, ..encoding };
    // Minimised (0×0) isn't a new size: the window comes back as it was.
    if size.0 == 0
        || size.1 == 0
        || ffmpeg_args::encoded_size(target, video) == ffmpeg_args::encoded_size(encoding, video)
    {
        *pending = None;
        return false;
    }
    match *pending {
        Some((seen, since)) if seen == size => now.duration_since(since) >= RESIZE_SETTLE,
        _ => {
            *pending = Some((size, now));
            false
        }
    }
}

fn start_recorder(ffmpeg: &Path, active: &mut Active) {
    active.pending_resize = None;
    let start = ring::next_index(&ring::scan(&active.dir).unwrap_or_default());
    active.epoch_from = start;
    match Recorder::start(ffmpeg, &active.window, &active.settings, &active.dir, start) {
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
    let from = ring::scan(&active.dir).ok().and_then(|s| s.last().map(|s| s.index)).unwrap_or(0).max(active.epoch_from);
    tracing::info!("full-session recording started at segment {from}");
    let _ = std::fs::write(active.dir.join(SESSION_FILE), from.to_string());
    active.session_from = Some(from);
    active.session_since = Some(unix_now());
}

/// Where recordings are saved and how big they may get in total.
#[derive(Debug, Clone, PartialEq, Eq)]
struct StoragePolicy {
    output_dir: PathBuf,
    limit: Option<u64>,
    keep_named: bool,
}

impl StoragePolicy {
    fn of(cfg: &Config) -> Self {
        Self {
            output_dir: cfg.output_dir.clone(),
            limit: cfg.storage_limit_bytes(),
            keep_named: cfg.keep_named_recordings,
        }
    }
}

/// The live policy, shared with export threads so one that finishes after
/// a settings change enforces the new limit/folder, not the old one.
type SharedStoragePolicy = Arc<std::sync::Mutex<StoragePolicy>>;

/// Deletes the oldest saved recordings until they fit under the limit.
fn enforce_storage_limit(policy: &SharedStoragePolicy, sink: &EventSink) {
    // Saves finishing together could both get here; one pass at a time.
    static RUNNING: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = RUNNING.lock().unwrap_or_else(|e| e.into_inner());
    let policy = policy.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let Some(limit) = policy.limit else { return };

    let recs = storage::scan(&policy.output_dir);
    let plan = storage::plan_cleanup(&recs, limit, policy.keep_named);
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

#[cfg(test)]
mod tests {
    use super::*;
    use ptv_core::config::Resolution;

    fn temp_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("ptv-sup-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    const ENCODING: CaptureTarget = CaptureTarget { hwnd: 1, width: 1658, height: 1030 };

    /// Feeds `sizes` one second apart; returns the call that said restart.
    fn first_restart(video: &VideoConfig, sizes: &[(u32, u32)]) -> Option<usize> {
        let start = Instant::now();
        let mut pending = None;
        sizes.iter().enumerate().position(|(i, &size)| {
            resize_settled(&mut pending, ENCODING, size, video, start + Duration::from_secs(i as u64))
        })
    }

    #[test]
    fn resize_restarts_once_the_new_size_holds() {
        let v = VideoConfig::default();
        // Seen at 0 s, still there at 1 s, settled at 2 s.
        assert_eq!(first_restart(&v, &[(1900, 1030); 4]), Some(2));
    }

    #[test]
    fn resize_waits_while_the_window_is_being_dragged() {
        let v = VideoConfig::default();
        let drag = [(1700, 1030), (1750, 1030), (1800, 1030), (1900, 1030), (1900, 1030), (1900, 1030)];
        assert_eq!(first_restart(&v, &drag), Some(5), "the timer restarts at each new size");
    }

    #[test]
    fn resize_ignores_going_back_minimising_and_same_output() {
        let v = VideoConfig::default();
        // Back to the encoded size before it settled.
        assert_eq!(first_restart(&v, &[(1900, 1030), (1658, 1030), (1658, 1030), (1658, 1030)]), None);
        // Minimised windows report 0×0.
        assert_eq!(first_restart(&v, &[(0, 0); 4]), None);
        // Minimising mid-change starts the wait over.
        assert_eq!(first_restart(&v, &[(1900, 1030), (0, 0), (1900, 1030), (1900, 1030), (1900, 1030)]), Some(4));
        // Native rounds to even, so an odd extra pixel encodes the same.
        assert_eq!(first_restart(&v, &[(1659, 1031); 4]), None);
        // A fixed height with the same aspect ratio encodes the same.
        let p1080 = VideoConfig { resolution: Resolution::P1080, ..Default::default() };
        assert_eq!(first_restart(&p1080, &[(3316, 2060); 4]), None);
        assert_eq!(first_restart(&p1080, &[(1900, 1030); 4]), Some(2));
    }

    #[test]
    fn live_buffers_are_locked_abandoned_ones_are_not() {
        let dir = temp_dir("lock");
        assert!(!buffer_in_use(&dir), "no lock file: a pre-lock leftover");
        let lock = lock_buffer(&dir).unwrap();
        assert!(buffer_in_use(&dir), "held by a running process");
        drop(lock);
        assert!(!buffer_in_use(&dir), "process gone: lock file left behind but free");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn storage_limit_uses_the_policy_current_when_it_runs() {
        let out = temp_dir("storage");
        let game = out.join("RuneLite");
        std::fs::create_dir_all(&game).unwrap();
        for (i, day) in ["01", "02", "03"].iter().enumerate() {
            let p = game.join(format!("RuneLite_2026-10-{day}_10-00-00.mp4"));
            std::fs::write(&p, [0u8; 10]).unwrap();
            // Distinct, increasing modification times.
            let t = std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(1_000 + i as u64);
            std::fs::File::options().write(true).open(&p).unwrap().set_modified(t).unwrap();
        }
        let events = Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink: EventSink = {
            let events = events.clone();
            Arc::new(move |e| events.lock().unwrap().push(e))
        };

        // An export captured the policy while there was no limit…
        let shared: SharedStoragePolicy =
            Arc::new(std::sync::Mutex::new(StoragePolicy { output_dir: out.clone(), limit: None, keep_named: false }));
        let captured_by_export = shared.clone();
        // …then the user set a limit (20 bytes) before it finished.
        *shared.lock().unwrap() = StoragePolicy { output_dir: out.clone(), limit: Some(20), keep_named: false };
        enforce_storage_limit(&captured_by_export, &sink);

        let left: Vec<String> =
            std::fs::read_dir(&game).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        assert_eq!(left.len(), 2, "the oldest is deleted under the *new* limit: {left:?}");
        assert!(!left.iter().any(|n| n.contains("10-01")));
        assert!(matches!(events.lock().unwrap().as_slice(), [Event::Cleaned { count: 1, bytes: 10 }]));
        std::fs::remove_dir_all(&out).unwrap();
    }
}
