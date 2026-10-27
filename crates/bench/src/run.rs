//! One benchmark run: start the recorder and the test game, let it settle,
//! measure, save a clip, then check the clip.

use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::analyze::{self, ClipReport};
use crate::env::Environment;
use crate::frame_log::{self, FrameLog};
use crate::playerstv::Playerstv;
use crate::recorders::{self, Recorder, Settings};
use crate::sample::{self, Sample, Sampler};
use crate::stats::Summary;
use crate::win::{procs, window};
use crate::{clips, input};

pub struct RunOptions {
    pub recorder: Recorder,
    pub round: u32,
    pub settings: Settings,
    pub warmup: Duration,
    pub measure: Duration,
    pub gpu_load: u32,
    /// Run the game in a window this size instead of borderless fullscreen.
    pub windowed: Option<(u32, u32)>,
    pub game_exe: PathBuf,
    pub playerstv_exe: PathBuf,
    pub ffmpeg: PathBuf,
    /// The suite's results folder; this run writes into a subfolder.
    pub out_dir: PathBuf,
    /// Where the recorder saves clips (playerstv's is set automatically).
    pub clips_dir: Option<PathBuf>,
    /// Overrides the recorder's usual save hotkey.
    pub hotkey: Option<String>,
    /// Show setup checklists and wait for Enter before manual recorders.
    pub interactive: bool,
    pub environment: Environment,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunResult {
    pub recorder: Recorder,
    pub round: u32,
    pub started: String,
    pub settings: Settings,
    pub gpu_load: u32,
    pub warmup_s: f64,
    pub measure_s: f64,
    pub environment: Environment,
    /// The setup steps the person running it confirmed.
    pub checklist: Vec<String>,
    pub warnings: Vec<String>,
    /// Medians over the measured samples.
    pub metrics: Sample,
    pub samples: Vec<Sample>,
    /// Time between the game's frames while measuring.
    pub frame_interval_ms: Option<Summary>,
    /// The game's own time per frame while measuring.
    pub game_work_ms: Option<Summary>,
    /// From pressing the hotkey to the clip being complete on disk.
    pub save_ms: Option<f64>,
    pub clip: Option<ClipReport>,
}

impl RunResult {
    pub fn file_name(recorder: Recorder, round: u32) -> String {
        format!("{}-r{round}.json", slug(recorder))
    }
}

const NO_SAMPLES: &str = "no samples were taken while measuring; measure for longer (--seconds)";

pub fn slug(recorder: Recorder) -> String {
    format!("{recorder:?}").to_lowercase()
}

pub fn run(o: &RunOptions) -> Result<RunResult> {
    let run_dir = o.out_dir.join(format!("{}-r{}", slug(o.recorder), o.round));
    std::fs::create_dir_all(&run_dir)?;
    let started = chrono::Local::now().to_rfc3339();
    eprintln!("== {} (round {})", o.recorder.label(), o.round);

    wait_for_others_to_close(o)?;
    let mut warnings = Vec::new();

    let checklist = o.recorder.checklist(&o.settings);
    if !checklist.is_empty() && o.interactive {
        eprintln!("Set up {} like this:", o.recorder.label());
        for step in &checklist {
            eprintln!("  - {step}");
        }
        eprintln!("Press Enter when it's ready.");
        std::io::stdin().read_line(&mut String::new())?;
    }

    let ptv = match o.recorder {
        Recorder::Playerstv => Some(Playerstv::start(&o.playerstv_exe, &run_dir, &o.settings)?),
        _ => None,
    };
    let measured = measure(o, &run_dir, ptv.as_ref());
    if let Some(p) = ptv {
        if let Err(e) = p.stop() {
            warnings.push(format!("{e:#}"));
        }
    }
    let m = measured?;
    warnings.extend(m.warnings);

    let result = RunResult {
        recorder: o.recorder,
        round: o.round,
        started,
        settings: o.settings,
        gpu_load: o.gpu_load,
        warmup_s: o.warmup.as_secs_f64(),
        measure_s: o.measure.as_secs_f64(),
        environment: o.environment.clone(),
        checklist,
        warnings,
        metrics: sample::medians(&m.samples).context(NO_SAMPLES)?,
        samples: m.samples,
        frame_interval_ms: m.frame_interval_ms,
        game_work_ms: m.game_work_ms,
        save_ms: m.save_ms,
        clip: m.clip,
    };
    let path = o.out_dir.join(RunResult::file_name(o.recorder, o.round));
    std::fs::write(&path, serde_json::to_string_pretty(&result)?)?;
    eprintln!("   saved {}", path.display());
    Ok(result)
}

/// A recorder's footprint while no game is running.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IdleResult {
    pub recorder: Recorder,
    pub started: String,
    pub measure_s: f64,
    pub environment: Environment,
    pub metrics: Sample,
    pub samples: Vec<Sample>,
}

/// Measures `o.recorder` with no game for `o.measure`, after `o.warmup`.
pub fn idle(o: &RunOptions) -> Result<IdleResult> {
    std::fs::create_dir_all(&o.out_dir)?;
    let started = chrono::Local::now().to_rfc3339();
    eprintln!("== {} idle", o.recorder.label());
    let run_dir = o.out_dir.join(format!("idle-{}", slug(o.recorder)));
    std::fs::create_dir_all(&run_dir)?;
    wait_for_others_to_close(o)?;
    let ptv = match o.recorder {
        Recorder::Playerstv => Some(Playerstv::start(&o.playerstv_exe, &run_dir, &o.settings)?),
        r if o.interactive && !r.automatic() => {
            eprintln!("Start {} the way you'd normally leave it running, then press Enter.", r.label());
            std::io::stdin().read_line(&mut String::new())?;
            None
        }
        _ => None,
    };
    std::thread::sleep(o.warmup);
    let sampler = Sampler::start(o.recorder.processes().iter().map(|s| s.to_string()).collect())?;
    std::thread::sleep(o.measure);
    let samples = sampler.finish();
    if let Some(p) = ptv {
        p.stop()?;
    }
    let samples = samples?;
    let result = IdleResult {
        recorder: o.recorder,
        started,
        measure_s: o.measure.as_secs_f64(),
        environment: o.environment.clone(),
        metrics: sample::medians(&samples).context(NO_SAMPLES)?,
        samples,
    };
    let path = o.out_dir.join(format!("idle-{}.json", slug(o.recorder)));
    std::fs::write(&path, serde_json::to_string_pretty(&result)?)?;
    eprintln!("   saved {}", path.display());
    Ok(result)
}

/// Waits until no recorder other than `o.recorder` is active: one left
/// running from an earlier run would be counted in this run's numbers, and
/// in the baseline's. Without `o.interactive` it fails instead.
fn wait_for_others_to_close(o: &RunOptions) -> Result<()> {
    loop {
        let running: Vec<String> = procs::list()?.into_iter().map(|p| p.exe).collect();
        if o.recorder == Recorder::Playerstv && running.iter().any(|e| e == "playerstv.exe") {
            bail!("playerstv is already running. Quit it from the tray first: it would hold the save hotkey.");
        }
        let others = recorders::others_running(o.recorder, &running);
        if others.is_empty() {
            return Ok(());
        }
        let others = others.join(", ");
        if !o.interactive {
            bail!("{others} is running and would skew the numbers. Close it first.");
        }
        eprintln!("{others} is still running and would skew the numbers. Close it, then press Enter.");
        std::io::stdin().read_line(&mut String::new())?;
    }
}

struct Measured {
    samples: Vec<Sample>,
    frame_interval_ms: Option<Summary>,
    game_work_ms: Option<Summary>,
    save_ms: Option<f64>,
    clip: Option<ClipReport>,
    warnings: Vec<String>,
}

fn measure(o: &RunOptions, run_dir: &Path, ptv: Option<&Playerstv>) -> Result<Measured> {
    let mut warnings = Vec::new();
    let log_path = run_dir.join("frames.csv");
    // Long enough to outlive a slow save; it's closed as soon as we're done.
    let game_seconds = (o.warmup + o.measure).as_secs() + 300;
    let mut game = Game::start(o, &log_path, game_seconds)?;
    window::focus_game(Duration::from_secs(10))?;

    if let Some(p) = ptv {
        p.wait_recording(Duration::from_secs(30))?;
    }
    eprintln!("   warming up for {} s", o.warmup.as_secs());
    std::thread::sleep(o.warmup);

    eprintln!("   measuring for {} s", o.measure.as_secs());
    let patterns = o.recorder.processes().iter().map(|s| s.to_string()).collect();
    let sampler = Sampler::start(patterns)?;
    let from_us = frame_log::unix_us_now();
    std::thread::sleep(o.measure);
    let to_us = frame_log::unix_us_now();
    let samples = sampler.finish()?;
    if !window::game_has_focus() {
        warnings.push("the test game wasn't the foreground window by the end of measuring".into());
    }

    let saved = match o.recorder {
        Recorder::None => None,
        r => {
            let dir = match ptv {
                Some(p) => p.clips_dir.clone(),
                None => o
                    .clips_dir
                    .clone()
                    .or_else(|| r.default_clips_dir())
                    .with_context(|| format!("pass --clips-dir: where does {} save clips?", r.label()))?,
            };
            let hotkey = o.hotkey.as_deref().or(r.hotkey()).context("no save hotkey")?;
            let before = clips::snapshot(&dir);
            let since = Instant::now();
            input::press(hotkey)?;
            eprintln!("   pressed {hotkey}, waiting for the clip in {}", dir.display());
            match clips::wait_for_new(&dir, &before, since, Duration::from_secs(120)) {
                Ok(found) => Some(found),
                Err(e) => {
                    warnings.push(format!("{e:#}"));
                    None
                }
            }
        }
    };

    game.close()?;
    let log = FrameLog::read(&log_path)?;
    let interval = Summary::of(&log.intervals_ms(from_us, to_us));
    let work: Vec<f64> = {
        let (from, to) = (from_us.saturating_sub(log.t0_unix_us), to_us.saturating_sub(log.t0_unix_us));
        log.frames.iter().filter(|f| (from..=to).contains(&f.present_us)).map(|f| f.work_us as f64 / 1000.0).collect()
    };

    let clip = match &saved {
        Some((path, _)) => {
            eprintln!("   checking {}", path.display());
            match analyze::analyze(&o.ffmpeg, path, log.pattern, Some(&log.frames)) {
                Ok(report) => Some(report),
                Err(e) => {
                    warnings.push(format!("couldn't analyze the clip: {e:#}"));
                    None
                }
            }
        }
        None => None,
    };

    Ok(Measured {
        samples,
        frame_interval_ms: interval,
        game_work_ms: Summary::of(&work),
        save_ms: saved.map(|(_, took)| took.as_secs_f64() * 1000.0),
        clip,
        warnings,
    })
}

/// The test game process; closed (or killed) when dropped.
struct Game(Option<Child>);

impl Game {
    fn start(o: &RunOptions, log: &Path, seconds: u64) -> Result<Self> {
        let mut cmd = Command::new(&o.game_exe);
        cmd.args(["--fps", &o.settings.fps.to_string(), "--seconds", &seconds.to_string()])
            .args(["--gpu-load", &o.gpu_load.to_string()])
            .arg("--log")
            .arg(log);
        if let Some((w, h)) = o.windowed {
            cmd.args(["--windowed", &format!("{w}x{h}")]);
        }
        if o.settings.audio {
            cmd.arg("--tone");
        }
        let child = cmd.spawn().with_context(|| format!("starting {}", o.game_exe.display()))?;
        Ok(Self(Some(child)))
    }

    /// Closes the window and waits for the game to write its log.
    fn close(&mut self) -> Result<()> {
        let Some(mut child) = self.0.take() else { return Ok(()) };
        window::close_game();
        let deadline = Instant::now() + Duration::from_secs(15);
        while Instant::now() < deadline {
            if let Some(status) = child.try_wait()? {
                if !status.success() {
                    bail!("the test game exited with {status}");
                }
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let _ = child.kill();
        bail!("the test game didn't close within 15 s");
    }
}

impl Drop for Game {
    fn drop(&mut self) {
        if let Some(mut child) = self.0.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}
