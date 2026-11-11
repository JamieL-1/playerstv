//! Background thread that owns the recording state machine:
//! wait for a game → start the encoder → keep the buffer trimmed → export
//! clips/sessions on request → tear down when the game closes.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::Local;
use crossbeam_channel::{Receiver, RecvTimeoutError};
use ptv_capture::detect::{self, GameWindow};
use ptv_common::sync::lock;
use ptv_core::clip::{self, ExportKind};
use ptv_core::config::{Config, VideoConfig};
use ptv_core::ffmpeg_args::{self, CaptureTarget};
use ptv_core::profile::{safe_name, ProfileMatcher};
use ptv_core::ring::{self, Container, Segment};
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
/// Holds the path of the recording a closed game's session is being saved to.
const SAVING_FILE: &str = "saving.txt";
/// A full session that ends on its own this soon after the game was found
/// (the encoder restarting, the game or playerstv closing) is dropped, not
/// saved. That footage is the game settling as it launches (League's window
/// opens square, then resizes), not play.
const LAUNCH_SETTLE: Duration = Duration::from_secs(10);
/// How often old segments are deleted. A segment only becomes old enough
/// when a new one starts, so looking more often than that finds nothing
/// (and a long full session's buffer is thousands of files to list).
const PRUNE_INTERVAL: Duration = Duration::from_secs(ring::SEGMENT_SECONDS as u64);
/// A clip takes the segment still being written once it's this old. A
/// younger one may not hold a first packet of every track yet, which ffmpeg
/// needs to read it. Costs a clip at most this long, once in four saves.
const LIVE_SEGMENT_MIN_AGE: Duration = Duration::from_millis(500);
/// When quitting, saves are waited for as long as they keep writing; one
/// that writes nothing for this long is given up on.
const EXPORT_STALL: Duration = Duration::from_secs(120);

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
    /// A window size or fullscreen state that would record differently from
    /// the running encoder, and when it was first seen.
    pending_resize: Option<(CaptureTarget, Instant)>,
    /// See [`Recorder::low_fps`].
    low_fps: Option<u32>,
    /// When the game was found.
    detected: Instant,
}

pub struct Supervisor {
    cfg: Config,
    matchers: Vec<ProfileMatcher>,
    ffmpeg: PathBuf,
    sink: EventSink,
    active: Option<Active>,
    exports_in_flight: Arc<AtomicUsize>,
    /// Goes up whenever a running save's output grows: see [`EXPORT_STALL`].
    export_progress: Arc<AtomicU64>,
    storage: SharedStoragePolicy,
    last_scan: Option<Instant>,
    last_prune: Option<Instant>,
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
            export_progress: Arc::new(AtomicU64::new(0)),
            last_scan: None,
            last_prune: None,
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
        self.stop_game(Stop::Auto);
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
                self.stop_game(Stop::Auto);
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
                        active.recorder.take().expect("checked above").stop();
                    }
                }
                let wants_encoder = wants_encoder(&active);
                if active.recorder.is_none() && wants_encoder && Instant::now() >= active.retry_at {
                    self.restart_recorder(&mut active);
                } else if window_resized(&mut active) {
                    tracing::info!("{} window resized, restarting encoder", active.window.profile.name);
                    active.failures = 0;
                    self.restart_recorder(&mut active);
                }
                watch_frame_rate(&mut active);
                if self.last_prune.is_none_or(|t| t.elapsed() >= PRUNE_INTERVAL)
                    && self.exports_in_flight.load(Ordering::SeqCst) == 0
                {
                    self.last_prune = Some(Instant::now());
                    if wants_encoder || active.recorder.is_some() {
                        prune(&active.dir, active.settings.buffer_seconds, active.session_from);
                    } else {
                        // Buffer off and idle: nothing to keep once saves are done.
                        clear_segments(&active.dir);
                    }
                }
                self.active = Some(active);
            }
        }
        self.publish_status();
    }

    /// Starts a fresh encoder for an already-running game. A full session in
    /// progress is saved (unless it's only just started) and continued in a
    /// new file, since the new encoder's output may not be concatenable with
    /// the old one's.
    ///
    /// The new encoder starts while the old one is still finishing its last
    /// segment, so the footage lost in between is the longer of the two
    /// waits rather than their sum.
    fn restart_recorder(&mut self, active: &mut Active) {
        let mut old = active.recorder.take();
        if let Some(rec) = &mut old {
            rec.request_stop();
        }
        // The session so far belongs to the old encoder: its file ends
        // where the new encoder's segments begin. session.txt stays until
        // `begin_session` rewrites it, so a crash in between loses nothing.
        let session_from = active.session_from.take();
        active.session_since = None;
        let old_titles = active.audio_titles.clone();
        start_recorder(&self.ffmpeg, active, old.is_some());
        if let Some(rec) = old {
            rec.wait_stopped();
        }
        let Some(from) = session_from else { return };
        let segs = session_segments(&active.dir, from, Some(active.epoch_from));
        if just_launched(active) {
            tracing::info!("dropped the full session recorded while {} was launching", active.window.profile.name);
        } else if !segs.is_empty() {
            self.save_session(active, segs, old_titles, Cleanup::None);
        }
        begin_session(active);
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
            low_fps: None,
            detected: Instant::now(),
        };
        if active.settings.record_full_session {
            start_session(&self.ffmpeg, &mut active);
        } else if active.settings.replay_buffer {
            start_recorder(&self.ffmpeg, &mut active, false);
        } else {
            tracing::info!("{} replay buffer is off, idle until a full session starts", active.window.profile.name);
        }
        self.active = Some(active);
    }

    fn stop_game(&mut self, why: Stop) {
        let Some(mut active) = self.active.take() else { return };
        if let Some(rec) = active.recorder.take() {
            rec.stop();
        }
        let dir = active.dir.clone();
        if why == Stop::Auto && just_launched(&active) {
            drop_session(&mut active);
        }
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
            let mut current = lock(&self.storage);
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
            self.stop_game(Stop::ByUser); // saves a full session if one was running
            return;
        }
        // Only this game's settings matter: another game's overrides
        // changing mustn't restart (and split) this recording.
        let settings = self.cfg.for_game(&active.window.profile.id);
        let restart = settings.capture_changed(&active.settings);
        let session_turned_on = settings.record_full_session && !active.settings.record_full_session;
        active.settings = settings;
        if !wants_encoder(&active) {
            // Buffer turned off with no session recording: stop writing now.
            if let Some(rec) = active.recorder.take() {
                tracing::info!("{} replay buffer turned off, stopping encoder", active.window.profile.name);
                rec.stop();
            }
            active.last_error = None;
            active.failures = 0;
        } else if restart {
            tracing::info!("capture settings changed, restarting encoder");
            active.failures = 0;
            self.restart_recorder(&mut active);
        }
        // Turned back on: the next tick starts the encoder.
        if session_turned_on && active.session_from.is_none() {
            start_session(&self.ffmpeg, &mut active);
        }
        self.active = Some(active);
    }

    fn save_clip(&mut self) {
        let Some(active) = &self.active else {
            self.emit(Event::Error("No game running, so there's nothing to clip.".into()));
            return;
        };
        if !active.settings.replay_buffer {
            let game = &active.window.profile.name;
            return self.emit(Event::Error(format!(
                "The replay buffer is off for {game}, so there's nothing to clip. Turn it on in the game's settings."
            )));
        }
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
        self.spawn_export(source, out, titles, ExportKind::Clip, Cleanup::None);
    }

    fn toggle_session(&mut self) {
        let Some(mut active) = self.active.take() else {
            self.emit(Event::Error("No game running. Session recording starts when playerstv detects a game.".into()));
            return;
        };
        if active.session_from.is_some() {
            if !active.settings.replay_buffer {
                // Nothing else needs the encoder: stop it first so the
                // session's last segment is complete, like a closing game.
                if let Some(rec) = active.recorder.take() {
                    rec.stop();
                }
                active.last_error = None;
                active.failures = 0;
            }
            self.finish_session(&mut active, false);
        } else {
            start_session(&self.ffmpeg, &mut active);
        }
        self.active = Some(active);
        self.publish_status();
    }

    /// Exports everything since the session started. With `cleanup`, the
    /// buffer folder is removed once it's saved (the game has closed).
    fn finish_session(&mut self, active: &mut Active, cleanup: bool) {
        let Some(from) = active.session_from.take() else { return };
        active.session_since = None;
        let segs = session_segments(&active.dir, from, None);
        let cleanup = if !cleanup {
            let _ = std::fs::remove_file(active.dir.join(SESSION_FILE));
            Cleanup::None
        } else if segs.is_empty() {
            Cleanup::Always(active.dir.clone())
        } else {
            // session.txt stays in the buffer until the file is written. If
            // playerstv is killed first, or the save fails (a full disk),
            // the next start finds the session and saves it then.
            Cleanup::WhenSaved(active.dir.clone())
        };
        self.save_session(active, segs, active.audio_titles.clone(), cleanup);
    }

    fn save_session(&self, active: &Active, segs: Vec<Segment>, titles: Vec<String>, cleanup: Cleanup) {
        let out =
            clip::output_path(&self.cfg.output_dir, &active.window.profile.name, ExportKind::Session, Local::now());
        self.spawn_export(ExportSource::Segments(segs), out, titles, ExportKind::Session, cleanup);
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
        cleanup: Cleanup,
    ) {
        let ffmpeg = self.ffmpeg.clone();
        let in_flight = self.exports_in_flight.clone();
        let progress = self.export_progress.clone();
        let sink = self.sink.clone();
        let storage = self.storage.clone();
        // A session saved while its game is still running is a big copy
        // that nobody is waiting on: it gives way to the game and to the
        // recording. Clips are small and wanted now; once the game has
        // closed there's nothing left to give way to.
        let background = kind == ExportKind::Session && matches!(cleanup, Cleanup::None);
        // Counted from now so pruning pauses while we wait for the segment.
        in_flight.fetch_add(1, Ordering::SeqCst);
        std::thread::spawn(move || {
            let started = Instant::now();
            let run = export::Run {
                background,
                on_progress: &|| {
                    progress.fetch_add(1, Ordering::Relaxed);
                },
            };
            let out = clip::unique_path(out);
            if let Cleanup::WhenSaved(dir) = &cleanup {
                // So a start after being killed mid-save can remove the
                // unfinished file before saving the session again.
                let _ = std::fs::write(dir.join(SAVING_FILE), out.to_string_lossy().as_bytes());
            }
            let save = |segs: &[Segment]| export::export(&ffmpeg, segs, out.clone(), &titles, kind, &run);
            let mut result = save(&source.resolve(true));
            if let (Err(e), true) = (&result, source.reads_live_segment()) {
                // Not seen in testing, but the segment being written is the
                // one thing here ffmpeg didn't finish: try again without it.
                tracing::warn!("saving with the segment still being written failed ({e:#}); waiting for it to end");
                result = save(&source.resolve(false));
            }
            let still_exporting = in_flight.fetch_sub(1, Ordering::SeqCst) > 1;
            let saved = result.is_ok();
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
            match cleanup {
                Cleanup::None => {}
                Cleanup::WhenSaved(dir) if !saved => {
                    tracing::warn!("keeping {} so the session can be saved at the next start", dir.display());
                }
                Cleanup::WhenSaved(dir) | Cleanup::Always(dir) => delete_when_idle(dir, in_flight),
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
            remove_unfinished_save(&dir);
            let out = clip::output_path(&self.cfg.output_dir, &game, ExportKind::Session, Local::now());
            // Its second chance: the buffer goes whether or not this works.
            self.spawn_export(ExportSource::Segments(segs), out, titles, ExportKind::Session, Cleanup::Always(dir));
        }
    }

    /// Waits for saves in progress for as long as they keep writing. A long
    /// session on a slow disk takes minutes to copy; cutting it off after a
    /// fixed time left an unplayable file.
    fn wait_for_exports(&self) {
        let mut seen = self.export_progress.load(Ordering::Relaxed);
        let mut since = Instant::now();
        while self.exports_in_flight.load(Ordering::SeqCst) > 0 {
            std::thread::sleep(Duration::from_millis(100));
            let now = self.export_progress.load(Ordering::Relaxed);
            if now != seen {
                (seen, since) = (now, Instant::now());
            } else if since.elapsed() >= EXPORT_STALL {
                tracing::warn!("a save has written nothing for {EXPORT_STALL:?}, quitting without it");
                break;
            }
        }
    }

    fn status(&self) -> Status {
        match &self.active {
            None => Status::Waiting,
            Some(a) if a.recorder.is_none() && !wants_encoder(a) => {
                Status::Idle { game: a.window.profile.name.clone() }
            }
            Some(a) => match (&a.recorder, &a.last_error) {
                (None, Some(err)) => Status::Failing { game: a.window.profile.name.clone(), error: err.clone() },
                _ => Status::Recording {
                    game: a.window.profile.name.clone(),
                    since: a.recording_since,
                    session_since: a.session_since,
                    video: a.recorder.as_ref().map(|r| r.video_summary.clone()).unwrap_or_default(),
                    buffer_seconds: a.settings.buffer_seconds,
                    low_fps: a.low_fps,
                    replay_buffer: a.settings.replay_buffer,
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

/// What happens to a buffer folder after a save from it.
enum Cleanup {
    /// Nothing: the game is still running.
    None,
    /// Deleted once the save succeeds. If it fails, the folder (with its
    /// session.txt) is left for the next start to try again.
    WhenSaved(PathBuf),
    /// Deleted whether or not the save succeeds.
    Always(PathBuf),
}

enum ExportSource {
    Segments(Vec<Segment>),
    /// The newest `seconds` of footage ending with segment `last`, which
    /// was still being written when the hotkey was pressed. MPEG-TS is
    /// flushed to disk packet by packet, so it's read as it stands, up to
    /// the newest frame. A Matroska (AV1) segment can't be read half
    /// written, so there we wait for ffmpeg to close it. Segments before
    /// `first` belong to an earlier encoder and are left out.
    ClipUpTo {
        dir: PathBuf,
        first: u64,
        last: u64,
        seconds: u32,
    },
}

impl ExportSource {
    /// Whether [`ExportSource::resolve`] may hand back a segment that's
    /// still being written.
    fn reads_live_segment(&self) -> bool {
        match self {
            ExportSource::Segments(_) => false,
            ExportSource::ClipUpTo { dir, last, .. } => {
                let ts = dir.join(ring::segment_file_name(*last, Container::MpegTs));
                Container::MpegTs.readable_while_written() && ts.exists()
            }
        }
    }

    /// The segments to save. With `live`, the one still being written is
    /// taken as it stands where its format allows; without, it's waited for.
    fn resolve(&self, live: bool) -> Vec<Segment> {
        match self {
            ExportSource::Segments(s) => s.clone(),
            ExportSource::ClipUpTo { dir, first, last, seconds } => {
                let newest = dir.join(ring::segment_file_name(*last, Container::MpegTs));
                if live && self.reads_live_segment() {
                    let wait = live_segment_wait(segment_age(&newest));
                    if !wait.is_zero() && !ring::segment_exists(dir, last + 1) {
                        std::thread::sleep(wait);
                    }
                } else {
                    let deadline = Instant::now() + Duration::from_secs(ring::SEGMENT_SECONDS as u64 + 2);
                    while !ring::segment_exists(dir, last + 1) && Instant::now() < deadline {
                        std::thread::sleep(Duration::from_millis(100));
                    }
                }
                let segs: Vec<Segment> = ring::scan(dir)
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|s| (*first..=*last).contains(&s.index))
                    .collect();
                ring::select_for_clip(&segs, *seconds).to_vec()
            }
        }
    }
}

/// Deletes the recording a save from the buffer `dir` was writing when
/// playerstv was killed: it has no index, so it can't be played. Only ever
/// a file named like one of ours, at the path that save wrote down.
fn remove_unfinished_save(dir: &Path) {
    let Ok(path) = std::fs::read_to_string(dir.join(SAVING_FILE)) else { return };
    let path = Path::new(path.trim());
    if path.file_name().and_then(|n| n.to_str()).is_some_and(storage::is_recording_file) && path.is_file() {
        match std::fs::remove_file(path) {
            Ok(()) => tracing::info!("removed unfinished {}", path.display()),
            Err(e) => tracing::warn!("couldn't remove unfinished {}: {e}", path.display()),
        }
    }
}

/// How long ago ffmpeg started writing the segment at `path`, if known.
fn segment_age(path: &Path) -> Option<Duration> {
    std::fs::metadata(path).ok()?.created().ok()?.elapsed().ok()
}

/// How much longer to leave a segment of this age before reading it while
/// it's written: see [`LIVE_SEGMENT_MIN_AGE`]. An unknown age waits in full.
fn live_segment_wait(age: Option<Duration>) -> Duration {
    LIVE_SEGMENT_MIN_AGE.saturating_sub(age.unwrap_or_default())
}

/// The segments of a full session that began at segment `from`, up to but
/// not including `until` (the first segment of a newer encoder).
fn session_segments(dir: &Path, from: u64, until: Option<u64>) -> Vec<Segment> {
    ring::scan(dir)
        .unwrap_or_default()
        .into_iter()
        .filter(|s| s.index >= from && until.is_none_or(|end| s.index < end))
        .collect()
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

/// True once the window has settled at a size, or in or out of being played
/// fullscreen, such that it would record differently from the running
/// encoder. The capture source fixes its frame size and crop at start, so
/// without a restart a game that changes size after launch (RuneLite opening
/// its sidebar, say) ends up letterboxed or resampled; and a game that stops
/// being played fullscreen must stop being recorded by duplicating its
/// monitor.
fn window_resized(active: &mut Active) -> bool {
    let Some(rec) = &active.recorder else {
        active.pending_resize = None;
        return false;
    };
    let target = detect::capture_target(active.window.hwnd);
    resize_settled(&mut active.pending_resize, rec.target, target, &active.settings.video, Instant::now())
}

/// [`window_resized`]'s decision for the window as `target` describes it
/// now, while `encoding` is what the encoder was started with. `pending`
/// carries a change between calls.
fn resize_settled(
    pending: &mut Option<(CaptureTarget, Instant)>,
    encoding: CaptureTarget,
    target: CaptureTarget,
    video: &VideoConfig,
    now: Instant,
) -> bool {
    let duplicated = |t| ffmpeg_args::duplicated_output(t, video);
    // The monitor is being recorded and the game no longer has it to itself
    // (alt-tabbed, minimised, moved): go back to recording only the window
    // straight away, before more of what else is on the monitor is caught.
    if duplicated(encoding).is_some() && duplicated(target).is_none() {
        *pending = None;
        return true;
    }
    let records_as = |t| (ffmpeg_args::encoded_size(t, video), ffmpeg_args::capture_crop(t), duplicated(t));
    // Minimised (0×0) isn't a change: the window comes back as it was.
    if target.width == 0 || target.height == 0 || records_as(target) == records_as(encoding) {
        *pending = None;
        return false;
    }
    match *pending {
        Some((seen, since)) if seen == target => now.duration_since(since) >= RESIZE_SETTLE,
        _ => {
            *pending = Some((target, now));
            false
        }
    }
}

/// Starts an encoder. `beside_old` says the previous one is still writing
/// its last segment: it may start one more before it stops, so the new
/// encoder leaves that number free.
fn start_recorder(ffmpeg: &Path, active: &mut Active, beside_old: bool) {
    active.pending_resize = None;
    let start = ring::next_index(&ring::scan(&active.dir).unwrap_or_default()) + beside_old as u64;
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

/// Logs when the captured frame rate drops well below the target, and when
/// it recovers; the status carries it to the UI.
fn watch_frame_rate(active: &mut Active) {
    let target = active.settings.video.fps;
    let focused = detect::is_foreground(active.window.hwnd);
    let low = active.recorder.as_mut().and_then(|r| r.low_fps(target, focused));
    match (active.low_fps, low) {
        (None, Some(fps)) => tracing::warn!(
            "{} capturing {fps} of {target} fps over the last 30 s: the game is drawing fewer frames, or capture is falling behind",
            active.window.profile.name
        ),
        (Some(_), None) => tracing::info!("{} capture frame rate recovered", active.window.profile.name),
        _ => {}
    }
    active.low_fps = low;
}

/// Whether a full session ending on its own now would hold only the game's
/// launch: see [`LAUNCH_SETTLE`]. Measured from when the game was found, not
/// when the current file started, so the tail of a long session that just
/// restarted is kept.
fn just_launched(active: &Active) -> bool {
    active.detected.elapsed() < LAUNCH_SETTLE
}

/// Why recording of a game stops.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stop {
    /// The game closed or playerstv is quitting.
    Auto,
    /// The player turned capture of this game off: a session always saves.
    ByUser,
}

/// Forgets a full session in progress without saving it.
fn drop_session(active: &mut Active) {
    if active.session_from.take().is_some() && just_launched(active) {
        tracing::info!("dropped the full session recorded while {} was launching", active.window.profile.name);
    }
    active.session_since = None;
    let _ = std::fs::remove_file(active.dir.join(SESSION_FILE));
}

/// Whether this game needs an encoder running: for its replay buffer, or for
/// a full session in progress.
fn wants_encoder(active: &Active) -> bool {
    active.settings.replay_buffer || active.session_from.is_some()
}

/// Starts a full session, starting the encoder first if the game was idle
/// (replay buffer off), so the session begins at that encoder's first segment.
fn start_session(ffmpeg: &Path, active: &mut Active) {
    if active.recorder.is_none() {
        start_recorder(ffmpeg, active, false);
    }
    begin_session(active);
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
    let _guard = lock(&RUNNING);
    let policy = lock(policy).clone();
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

/// Deletes every segment, for a game whose replay buffer is off once
/// nothing needs its footage any more.
fn clear_segments(dir: &Path) {
    let Ok(segs) = ring::scan(dir) else { return };
    for seg in segs {
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

    const ENCODING: CaptureTarget = CaptureTarget { hwnd: 1, width: 1658, height: 1030, fullscreen_output: None };

    /// Feeds `targets` one second apart to an encoder started on `encoding`;
    /// returns the call that said restart.
    fn first_restart_from(encoding: CaptureTarget, video: &VideoConfig, targets: &[CaptureTarget]) -> Option<usize> {
        let start = Instant::now();
        let mut pending = None;
        targets.iter().enumerate().position(|(i, &target)| {
            resize_settled(&mut pending, encoding, target, video, start + Duration::from_secs(i as u64))
        })
    }

    /// [`first_restart_from`] for a windowed game going through `sizes`.
    fn first_restart(video: &VideoConfig, sizes: &[(u32, u32)]) -> Option<usize> {
        let targets: Vec<CaptureTarget> =
            sizes.iter().map(|&(width, height)| CaptureTarget { width, height, ..ENCODING }).collect();
        first_restart_from(ENCODING, video, &targets)
    }

    #[test]
    fn going_in_or_out_of_fullscreen_restarts() {
        let v = VideoConfig::default();
        let windowed = CaptureTarget { width: 2560, height: 1440, ..ENCODING };
        let fullscreen = CaptureTarget { fullscreen_output: Some(0), ..windowed };
        // Same size, but now it fills its monitor: duplicate the monitor.
        assert_eq!(first_restart_from(windowed, &v, &[fullscreen; 3]), Some(2));
        // Back to capturing the window the moment it stops being played
        // fullscreen, minimised included: the monitor now shows other things.
        assert_eq!(first_restart_from(fullscreen, &v, &[windowed; 3]), Some(0));
        let minimised = CaptureTarget { width: 0, height: 0, ..windowed };
        assert_eq!(first_restart_from(fullscreen, &v, &[minimised; 3]), Some(0));
        // At a scaled resolution both are recorded from the window anyway.
        let p1080 = VideoConfig { resolution: Resolution::P1080, ..Default::default() };
        assert_eq!(first_restart_from(windowed, &p1080, &[fullscreen; 3]), None);
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
        // An odd extra pixel encodes to the same size but needs cropping, or
        // gfxcapture would resample the frame to fit.
        assert_eq!(first_restart(&v, &[(1659, 1031); 4]), Some(2));
        // A fixed height with the same aspect ratio encodes the same.
        let p1080 = VideoConfig { resolution: Resolution::P1080, ..Default::default() };
        assert_eq!(first_restart(&p1080, &[(3316, 2060); 4]), None);
        assert_eq!(first_restart(&p1080, &[(1900, 1030); 4]), Some(2));
    }

    #[test]
    fn a_clip_reads_the_segment_being_written_once_it_has_every_track() {
        assert_eq!(live_segment_wait(Some(Duration::from_millis(100))), Duration::from_millis(400));
        assert_eq!(live_segment_wait(Some(Duration::from_secs(1))), Duration::ZERO);
        assert_eq!(live_segment_wait(None), LIVE_SEGMENT_MIN_AGE);

        let dir = temp_dir("live");
        for i in 3..=5 {
            std::fs::write(dir.join(ring::segment_file_name(i, Container::MpegTs)), b"x").unwrap();
        }
        let clip = ExportSource::ClipUpTo { dir: dir.clone(), first: 4, last: 5, seconds: 30 };
        assert!(clip.reads_live_segment());
        // Segment 5 is taken without waiting for a segment 6, and nothing
        // from before the current encoder (segment 3).
        let started = Instant::now();
        let indices: Vec<u64> = clip.resolve(true).iter().map(|s| s.index).collect();
        assert_eq!(indices, [4, 5]);
        assert!(started.elapsed() < Duration::from_secs(2), "waited for the segment to end");

        // AV1's Matroska segments can't be read half written.
        std::fs::write(dir.join(ring::segment_file_name(9, Container::Matroska)), b"x").unwrap();
        assert!(!ExportSource::ClipUpTo { dir: dir.clone(), first: 9, last: 9, seconds: 30 }.reads_live_segment());
        assert!(!ExportSource::Segments(Vec::new()).reads_live_segment());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_restarted_session_ends_where_the_new_encoder_begins() {
        let dir = temp_dir("session");
        for i in [2, 3, 4, 6, 7] {
            std::fs::write(dir.join(ring::segment_file_name(i, Container::MpegTs)), b"x").unwrap();
        }
        let indices = |from, until| session_segments(&dir, from, until).iter().map(|s| s.index).collect::<Vec<_>>();
        assert_eq!(indices(3, None), [3, 4, 6, 7]);
        // The new encoder started at 6, leaving 5 free for the old one.
        assert_eq!(indices(3, Some(6)), [3, 4]);
        assert_eq!(indices(6, Some(6)), [0u64; 0]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn only_our_own_unfinished_recording_is_removed() {
        let dir = temp_dir("unfinished");
        let ours = dir.join("RuneLite_session_2026-10-21_14-03-22.mp4");
        let other = dir.join("notes.mp4");
        std::fs::write(&ours, b"half").unwrap();
        std::fs::write(&other, b"keep").unwrap();

        std::fs::write(dir.join(SAVING_FILE), other.to_string_lossy().as_bytes()).unwrap();
        remove_unfinished_save(&dir);
        assert!(other.exists(), "not a playerstv recording name");

        std::fs::write(dir.join(SAVING_FILE), ours.to_string_lossy().as_bytes()).unwrap();
        remove_unfinished_save(&dir);
        assert!(!ours.exists());
        std::fs::remove_dir_all(&dir).unwrap();
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
