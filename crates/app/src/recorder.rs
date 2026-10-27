//! One running capture: an ffmpeg process writing buffer segments, plus the
//! audio feeders piping PCM into it.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Write};
use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use ptv_capture::audio::{self, AudioSource, PipeFeeder};
use ptv_capture::detect::{self, GameWindow};
use ptv_core::config::{AudioSourceKind, Config};
use ptv_core::ffmpeg_args::{self, AudioInput, CaptureTarget, RecordSpec};

pub use ptv_common::process::{bind_to_our_lifetime, CREATE_NO_WINDOW};
use ptv_common::sync::lock;

const STDERR_TAIL: usize = 40;
/// How long the captured frame rate is averaged over before it counts as low.
const RATE_WINDOW: Duration = Duration::from_secs(30);
/// Below this share of the target frame rate, capture counts as low.
/// Windows Graphics Capture can deliver as little as about 70% for reasons
/// outside our control (BENCHMARKS.md), so that isn't flagged; 0.4.1's
/// 17 of 60 fps would have been.
const LOW_RATE: f64 = 0.6;

pub struct Recorder {
    child: Child,
    stdin: Option<ChildStdin>,
    // Dropped after ffmpeg exits so their pipe writes fail cleanly.
    feeders: Vec<PipeFeeder>,
    stderr_tail: Arc<Mutex<VecDeque<String>>>,
    /// ffmpeg's latest frame count; 0 until it first reports.
    frames: Arc<AtomicU64>,
    rate: RateWatch,
    started: Instant,
    /// Titles of the audio tracks, in stream order.
    pub audio_titles: Vec<String>,
    /// What's being encoded, e.g. "1900×1030 · 120 fps · H.264 · High quality".
    pub video_summary: String,
    /// The window and its size when capture started. gfxcapture keeps that
    /// frame size, letterboxing the window if it's resized later.
    pub target: CaptureTarget,
}

impl Recorder {
    pub fn start(
        ffmpeg: &Path,
        window: &GameWindow,
        cfg: &Config,
        segment_dir: &Path,
        start_index: u64,
    ) -> Result<Self> {
        let a = &cfg.audio;
        // Audio counts from here, and ffmpeg is told this time so it puts
        // video on the same clock.
        let origin = Instant::now();
        let origin_unix_us = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_micros() as u64);
        let mut feeders = Vec::new();
        let mut inputs = Vec::new();
        if a.game_audio {
            let source = match a.source {
                AudioSourceKind::Game => AudioSource::Process { pid: window.pid },
                AudioSourceKind::Desktop => AudioSource::Desktop,
            };
            let f = PipeFeeder::start(source, audio::pipe_name("game"), origin)?;
            inputs.push(AudioInput {
                pipe_path: f.pipe_path().to_owned(),
                title: a.source.track_title().into(),
                mix_volume: a.game_volume,
            });
            feeders.push(f);
        }
        if a.mic {
            let f =
                PipeFeeder::start(AudioSource::Mic { device: a.mic_device.clone() }, audio::pipe_name("mic"), origin)?;
            inputs.push(AudioInput {
                pipe_path: f.pipe_path().to_owned(),
                title: "Mic".into(),
                mix_volume: a.mic_volume,
            });
            feeders.push(f);
        }

        let (width, height) = detect::client_size(window.hwnd);
        let target = CaptureTarget { hwnd: window.hwnd, width, height };
        let video_summary = ffmpeg_args::describe(&ffmpeg_args::encode_plan(target, &cfg.video), &cfg.video);
        let spec = RecordSpec {
            target,
            video: &cfg.video,
            audio: &inputs,
            mixed_track: a.mixed_track,
            audio_bitrate_kbps: a.bitrate_kbps,
            segment_dir,
            start_index,
            audio_origin_unix_us: origin_unix_us,
        };
        let args = ffmpeg_args::record_args(&spec);
        let audio_titles = ffmpeg_args::track_titles(&spec);
        tracing::info!("starting ffmpeg: {}", args.join(" "));

        let mut child = Command::new(ffmpeg)
            .args(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .with_context(|| format!("launching {}", ffmpeg.display()))?;
        bind_to_our_lifetime(&child);

        let stderr_tail = Arc::new(Mutex::new(VecDeque::new()));
        if let Some(stderr) = child.stderr.take() {
            let tail = stderr_tail.clone();
            std::thread::Builder::new().name("ffmpeg-stderr".into()).spawn(move || {
                for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                    tracing::debug!(target: "ffmpeg", "{line}");
                    let mut t = lock(&tail);
                    if t.len() == STDERR_TAIL {
                        t.pop_front();
                    }
                    t.push_back(line);
                }
            })?;
        }

        let frames = Arc::new(AtomicU64::new(0));
        if let Some(stdout) = child.stdout.take() {
            let frames = frames.clone();
            std::thread::Builder::new().name("ffmpeg-progress".into()).spawn(move || {
                for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                    if let Some(n) = line.strip_prefix("frame=").and_then(|n| n.trim().parse().ok()) {
                        frames.store(n, Ordering::Relaxed);
                    }
                }
            })?;
        }

        Ok(Self {
            stdin: child.stdin.take(),
            child,
            feeders,
            stderr_tail,
            frames,
            rate: RateWatch::default(),
            started: Instant::now(),
            audio_titles,
            video_summary,
            target,
        })
    }

    /// `Some(description)` once ffmpeg has exited.
    pub fn exited(&mut self) -> Option<String> {
        match self.child.try_wait() {
            Ok(Some(status)) => Some(format!("ffmpeg exited ({status}): {}", self.stderr_summary())),
            Ok(None) => None,
            Err(e) => Some(format!("ffmpeg wait failed: {e}")),
        }
    }

    pub fn uptime(&self) -> Duration {
        self.started.elapsed()
    }

    /// The captured frame rate, if it has stayed below [`LOW_RATE`] of
    /// `target` for [`RATE_WINDOW`]. Only time with the game `focused`
    /// counts: a game in the background may draw less, or nothing.
    pub fn low_fps(&mut self, target: u32, focused: bool) -> Option<u32> {
        if !focused {
            self.rate = RateWatch::default();
            return None;
        }
        // Timed now, not when ffmpeg last reported: if it stops reporting,
        // the rate must fall rather than hold at its last value.
        self.rate.observe((Instant::now(), self.frames.load(Ordering::Relaxed)));
        self.rate.fps().filter(|&fps| fps < target as f64 * LOW_RATE).map(|fps| fps.round() as u32)
    }

    fn stderr_summary(&self) -> String {
        let t = lock(&self.stderr_tail);
        t.iter().rev().take(5).rev().cloned().collect::<Vec<_>>().join(" | ")
    }

    /// Asks ffmpeg to finish the current segment and exit; kills it if it
    /// doesn't within a few seconds.
    pub fn stop(mut self) {
        for f in &self.feeders {
            f.signal_stop();
        }
        if let Some(mut stdin) = self.stdin.take() {
            let _ = stdin.write_all(b"q");
            let _ = stdin.flush();
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match self.child.try_wait() {
                Ok(Some(_)) | Err(_) => break,
                Ok(None) if Instant::now() >= deadline => {
                    tracing::warn!("ffmpeg did not stop in time, killing it");
                    let _ = self.child.kill();
                    let _ = self.child.wait();
                    break;
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            }
        }
        self.feeders.clear();
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        // Only reached without stop() on panics/early returns.
        if matches!(self.child.try_wait(), Ok(None)) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

/// Frame counts over the last [`RATE_WINDOW`].
#[derive(Default)]
struct RateWatch {
    samples: VecDeque<(Instant, u64)>,
}

impl RateWatch {
    fn observe(&mut self, sample: (Instant, u64)) {
        if self.samples.back().is_some_and(|&(t, _)| t >= sample.0) {
            return;
        }
        self.samples.push_back(sample);
        // Keep one sample at least a window old, so the window is covered.
        while self.samples.len() > 2 && sample.0.duration_since(self.samples[1].0) >= RATE_WINDOW {
            self.samples.pop_front();
        }
    }

    /// Frames per second over the window, once it's covered.
    fn fps(&self) -> Option<f64> {
        let (&(t0, f0), &(t1, f1)) = (self.samples.front()?, self.samples.back()?);
        let span = t1.duration_since(t0);
        (span >= RATE_WINDOW).then(|| f1.saturating_sub(f0) as f64 / span.as_secs_f64())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rate_needs_a_full_window_then_tracks_the_latest_one() {
        let t0 = Instant::now();
        let mut r = RateWatch::default();
        // 60 fps for 20 s: not enough to judge yet.
        for s in 0..=10 {
            r.observe((t0 + Duration::from_secs(s * 2), s * 120));
        }
        assert_eq!(r.fps(), None);
        // Then 17 fps for 40 s: the window holds only the slow part.
        for s in 1..=20 {
            r.observe((t0 + Duration::from_secs(20 + s * 2), 1200 + s * 34));
        }
        let fps = r.fps().unwrap();
        assert!((fps - 17.0).abs() < 0.5, "{fps}");
        // Repeated reports of the same sample are ignored.
        r.observe((t0 + Duration::from_secs(60), 1880));
        assert_eq!(r.samples.len(), 16);
    }

    #[test]
    fn rate_falls_when_ffmpeg_stops_reporting() {
        let t0 = Instant::now();
        let mut r = RateWatch::default();
        // Healthy for 30 s, then the count stops moving for 30 s.
        for s in 0..=60 {
            r.observe((t0 + Duration::from_secs(s), s.min(30) * 60));
        }
        assert_eq!(r.fps(), Some(0.0));
    }
}
