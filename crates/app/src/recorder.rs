//! One running capture: an ffmpeg process writing buffer segments, plus the
//! audio feeders piping PCM into it.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Write};
use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use ptv_capture::audio::{self, AudioSource, PipeFeeder};
use ptv_capture::detect::{self, GameWindow};
use ptv_core::config::{AudioSourceKind, Config};
use ptv_core::ffmpeg_args::{self, AudioInput, CaptureTarget, RecordSpec};

pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const STDERR_TAIL: usize = 40;

/// Ties a child process's lifetime to ours: every ffmpeg we start goes into a
/// job object that Windows kills when our last handle to it closes, so a
/// crash or `taskkill /F` of playerstv never leaves ffmpeg recording forever.
pub fn bind_to_our_lifetime(child: &Child) {
    use std::os::windows::io::AsRawHandle;
    use std::sync::OnceLock;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };

    // HANDLE isn't Send/Sync; store the raw value. Never closed on purpose.
    static JOB: OnceLock<Option<usize>> = OnceLock::new();
    let job = JOB.get_or_init(|| unsafe {
        let job = CreateJobObjectW(None, None).ok()?;
        let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            &info as *const _ as *const _,
            std::mem::size_of_val(&info) as u32,
        )
        .ok()?;
        Some(job.0 as usize)
    });
    if let Some(job) = job {
        let process = HANDLE(child.as_raw_handle());
        if let Err(e) = unsafe { AssignProcessToJobObject(HANDLE(*job as *mut _), process) } {
            tracing::warn!("couldn't bind ffmpeg to our lifetime: {e}");
        }
    }
}

pub struct Recorder {
    child: Child,
    stdin: Option<ChildStdin>,
    // Dropped after ffmpeg exits so their pipe writes fail cleanly.
    feeders: Vec<PipeFeeder>,
    stderr_tail: Arc<Mutex<VecDeque<String>>>,
    started: Instant,
    /// Titles of the audio tracks, in stream order.
    pub audio_titles: Vec<String>,
    /// What's being encoded, e.g. "1900×1030 · 120 fps · H.264 4:4:4 · High quality".
    pub video_summary: String,
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
        let mut feeders = Vec::new();
        let mut inputs = Vec::new();
        if a.game_audio {
            let source = match a.source {
                AudioSourceKind::Game => AudioSource::Process { pid: window.pid },
                AudioSourceKind::Desktop => AudioSource::Desktop,
            };
            let f = PipeFeeder::start(source, audio::pipe_name("game"))?;
            inputs.push(AudioInput {
                pipe_path: f.pipe_path().to_owned(),
                title: a.source.track_title().into(),
                mix_volume: a.game_volume,
            });
            feeders.push(f);
        }
        if a.mic {
            let f = PipeFeeder::start(AudioSource::Mic { device: a.mic_device.clone() }, audio::pipe_name("mic"))?;
            inputs.push(AudioInput { pipe_path: f.pipe_path().to_owned(), title: "Mic".into(), mix_volume: a.mic_volume });
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
                    let mut t = tail.lock().unwrap();
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
        let t = self.stderr_tail.lock().unwrap();
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
