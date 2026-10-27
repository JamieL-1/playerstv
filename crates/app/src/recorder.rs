//! One running capture: an ffmpeg process writing buffer segments, plus the
//! audio feeders piping PCM into it.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Write};
use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
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

pub struct Recorder {
    child: Child,
    stdin: Option<ChildStdin>,
    // Dropped after ffmpeg exits so their pipe writes fail cleanly.
    feeders: Vec<PipeFeeder>,
    stderr_tail: Arc<Mutex<VecDeque<String>>>,
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
            .stdout(Stdio::null())
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

        Ok(Self {
            stdin: child.stdin.take(),
            child,
            feeders,
            stderr_tail,
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
