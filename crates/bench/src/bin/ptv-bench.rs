//! Runs and reports the recorder benchmarks. See BENCHMARKS.md.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use clap::{Args, Parser, Subcommand};
use ptv_bench::analyze;
use ptv_bench::env::Environment;
use ptv_bench::frame_log::FrameLog;
use ptv_bench::pattern::Pattern;
use ptv_bench::recorders::{Recorder, Settings};
use ptv_bench::report;
use ptv_bench::run::{self, RunOptions};
use ptv_bench::win::window;
use ptv_core::config::VideoConfig;
use ptv_core::ffmpeg_args::{self, CaptureTarget};

#[derive(Parser)]
#[command(about = "Benchmarks playerstv and other replay recorders")]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Every recorder, several rounds, in a shuffled order each round, with
    /// a no-recorder baseline in every round. Writes a report at the end.
    Suite {
        /// Recorders to compare. The baseline is always added.
        #[arg(long, value_enum, value_delimiter = ',', default_value = "playerstv")]
        recorders: Vec<Recorder>,
        #[arg(long, default_value_t = 5)]
        rounds: u32,
        /// Also measure each recorder idle (no game) once.
        #[arg(long)]
        idle: bool,
        #[command(flatten)]
        common: Common,
    },
    /// One measured run of one recorder.
    Run {
        #[arg(long, value_enum)]
        recorder: Recorder,
        #[arg(long, default_value_t = 1)]
        round: u32,
        #[command(flatten)]
        common: Common,
    },
    /// A recorder's footprint with no game running.
    Idle {
        #[arg(long, value_enum)]
        recorder: Recorder,
        #[command(flatten)]
        common: Common,
    },
    /// Builds report.md and report.json from a results folder.
    Report { dir: PathBuf },
    /// Check a clip of the test game: dropped frames, quality, A/V sync.
    Analyze {
        clip: PathBuf,
        /// The frame log the game wrote while the clip was recorded.
        #[arg(long)]
        log: PathBuf,
    },
    /// Draw one of the test game's frames to an image file.
    Render {
        /// Output image, e.g. frame.png.
        out: PathBuf,
        #[arg(long, default_value_t = 0)]
        index: u64,
        #[arg(long, value_parser = parse_size, default_value = "1920x1080")]
        size: (u32, u32),
        #[arg(long, default_value_t = 60)]
        fps: u32,
    },
}

#[derive(Args)]
struct Common {
    #[arg(long, default_value_t = 60)]
    fps: u32,
    /// Replay buffer length every recorder is set to.
    #[arg(long, default_value_t = 30)]
    buffer: u32,
    /// Seconds to let the recorder settle before measuring.
    #[arg(long, default_value_t = 30)]
    warmup: u64,
    /// Seconds to measure.
    #[arg(long, default_value_t = 180)]
    seconds: u64,
    /// Extra GPU work per game frame (full-frame copies).
    #[arg(long, default_value_t = 0)]
    gpu_load: u32,
    /// Run the game in a window of this size (e.g. 1920x1080) instead of
    /// borderless over the primary monitor.
    #[arg(long, value_parser = parse_size)]
    windowed: Option<(u32, u32)>,
    /// Play the game's tone and record game audio. Audible.
    #[arg(long)]
    audio: bool,
    /// Results folder. Defaults to bench/results/<date-time>.
    #[arg(long)]
    out: Option<PathBuf>,
    /// Where a recorder saves clips, as RECORDER=FOLDER (repeatable), or a
    /// bare folder for every recorder.
    #[arg(long)]
    clips_dir: Vec<String>,
    /// A recorder's save hotkey when it isn't the one its checklist sets,
    /// as RECORDER=KEYS (repeatable), e.g. obs=Ctrl+F12.
    #[arg(long)]
    hotkey: Vec<String>,
    /// Don't show setup checklists or wait for Enter.
    #[arg(long)]
    yes: bool,
}

impl Common {
    fn clips_dir_for(&self, recorder: Recorder) -> Result<Option<PathBuf>> {
        Ok(per_recorder(&self.clips_dir, recorder)?.map(PathBuf::from))
    }

    fn hotkey_for(&self, recorder: Recorder) -> Result<Option<String>> {
        per_recorder(&self.hotkey, recorder)
    }
}

/// The value for `recorder` from `NAME=VALUE` entries; a bare `VALUE`
/// applies to every recorder.
fn per_recorder(entries: &[String], recorder: Recorder) -> Result<Option<String>> {
    use clap::ValueEnum;
    let mut found = None;
    for e in entries {
        match e.split_once('=') {
            Some((name, value)) => {
                let r = Recorder::from_str(name, true).map_err(|_| anyhow::anyhow!("unknown recorder {name:?}"))?;
                if r == recorder {
                    found = Some(value.to_owned());
                }
            }
            None if found.is_none() => found = Some(e.clone()),
            None => {}
        }
    }
    Ok(found)
}

fn parse_size(s: &str) -> Result<(u32, u32), String> {
    let (w, h) = s.split_once('x').ok_or("expected WIDTHxHEIGHT")?;
    Ok((w.parse().map_err(|_| "bad width")?, h.parse().map_err(|_| "bad height")?))
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    // Before any thread starts: the control pipe name comes from this.
    ptv_bench::playerstv::use_bench_instance();
    match cli.command {
        Cmd::Suite { recorders, rounds, idle, common } => suite(&recorders, rounds, idle, &common)?,
        Cmd::Run { recorder, round, common } => {
            let o = options(&common, recorder, round, &out_dir(&common), ptv_bench::env::collect())?;
            let result = run::run(&o)?;
            println!("{}", serde_json::to_string_pretty(&result.metrics)?);
        }
        Cmd::Idle { recorder, common } => {
            let o = options(&common, recorder, 0, &out_dir(&common), ptv_bench::env::collect())?;
            let result = run::idle(&o)?;
            println!("{}", serde_json::to_string_pretty(&result.metrics)?);
        }
        Cmd::Report { dir } => write_report(&dir)?,
        Cmd::Analyze { clip, log } => {
            let log = FrameLog::read(&log)?;
            let report = analyze::analyze(&ffmpeg(), &clip, log.pattern, Some(&log.frames))?;
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
        Cmd::Render { out, index, size: (w, h), fps } => render(&out, Pattern::new(w, h, fps), index)?,
    }
    Ok(())
}

fn suite(recorders: &[Recorder], rounds: u32, idle: bool, common: &Common) -> Result<()> {
    let out = out_dir(common);
    // Left-over runs would be mixed into this suite's report.
    if out.exists() && report::load(&out).ok().is_none_or(|(runs, idle)| !runs.is_empty() || !idle.is_empty()) {
        bail!("{} already holds results. Pick a new folder with --out.", out.display());
    }
    let environment = ptv_bench::env::collect();
    let mut lineup: Vec<Recorder> = recorders.iter().copied().filter(|&r| r != Recorder::None).collect();
    lineup.sort();
    lineup.dedup();
    lineup.insert(0, Recorder::None);
    eprintln!("results go to {}", out.display());

    let mut failures = Vec::new();
    if idle {
        for &r in lineup.iter().filter(|&&r| r != Recorder::None) {
            let o = options(common, r, 0, &out, environment.clone())?;
            if let Err(e) = run::idle(&o) {
                eprintln!("error: {} idle: {e:#}", r.label());
                failures.push(format!("{} idle: {e:#}", r.label()));
            }
        }
    }
    for round in 1..=rounds {
        let mut order = lineup.clone();
        shuffle(&mut order, round as u64);
        for r in order {
            let o = options(common, r, round, &out, environment.clone())?;
            if let Err(e) = run::run(&o) {
                eprintln!("error: {} round {round}: {e:#}", r.label());
                failures.push(format!("{} round {round}: {e:#}", r.label()));
            }
        }
    }
    if !failures.is_empty() {
        std::fs::write(out.join("failures.txt"), failures.join("\n"))?;
    }
    write_report(&out)?;
    if !failures.is_empty() {
        bail!("{} runs failed; see {}", failures.len(), out.join("failures.txt").display());
    }
    Ok(())
}

/// A fixed shuffle per round, so heat and background activity don't always
/// land on the same recorder, and a rerun uses the same order.
fn shuffle(items: &mut [Recorder], seed: u64) {
    let mut state = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    for i in (1..items.len()).rev() {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        items.swap(i, (state % (i as u64 + 1)) as usize);
    }
}

fn write_report(dir: &Path) -> Result<()> {
    let (runs, idle) = report::load(dir)?;
    if runs.is_empty() && idle.is_empty() {
        bail!("no results in {}", dir.display());
    }
    report::check_consistent(&runs, &idle)?;
    let report = report::build(&runs, idle);
    std::fs::write(dir.join("report.json"), serde_json::to_string_pretty(&report)?)?;
    let md = report::markdown(&report);
    std::fs::write(dir.join("report.md"), &md)?;
    println!("{md}");
    eprintln!("wrote {}", dir.join("report.md").display());
    Ok(())
}

/// Hands one raw frame to ffmpeg, which writes it in the format `out` names.
fn render(out: &Path, pattern: Pattern, index: u64) -> Result<()> {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let mut frame = vec![0u8; pattern.frame_bytes()];
    pattern.render(index, &mut frame);
    let mut child = Command::new(ffmpeg())
        .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "rawvideo", "-pix_fmt", "bgra"])
        .args(["-s", &format!("{}x{}", pattern.width, pattern.height), "-i", "-", "-frames:v", "1"])
        .arg(out)
        .stdin(Stdio::piped())
        .spawn()?;
    child.stdin.take().context("ffmpeg stdin")?.write_all(&frame)?;
    if !child.wait()?.success() {
        bail!("ffmpeg couldn't write {}", out.display());
    }
    Ok(())
}

fn out_dir(c: &Common) -> PathBuf {
    c.out.clone().unwrap_or_else(|| {
        PathBuf::from("bench").join("results").join(chrono::Local::now().format("%Y-%m-%d_%H-%M-%S").to_string())
    })
}

fn ffmpeg() -> PathBuf {
    ptv_common::paths::ffmpeg_exe()
}

fn beside_me(exe: &str) -> Result<PathBuf> {
    let me = std::env::current_exe()?;
    let path = me.parent().context("exe has no folder")?.join(exe);
    if !path.exists() {
        bail!("{} not found. Build it with `cargo build --release` first.", path.display());
    }
    Ok(path)
}

fn options(c: &Common, recorder: Recorder, round: u32, out: &Path, environment: Environment) -> Result<RunOptions> {
    let (width, height) = c.windowed.unwrap_or_else(window::primary_monitor_size);
    // The cap playerstv picks for High quality; others are set to match.
    let video = VideoConfig { fps: c.fps, ..Default::default() };
    let bitrate_mbps = ffmpeg_args::encode_plan(CaptureTarget { hwnd: 0, width, height }, &video).cap_mbps;
    Ok(RunOptions {
        recorder,
        round,
        settings: Settings { width, height, fps: c.fps, buffer_seconds: c.buffer, bitrate_mbps, audio: c.audio },
        warmup: Duration::from_secs(c.warmup),
        measure: Duration::from_secs(c.seconds),
        gpu_load: c.gpu_load,
        windowed: c.windowed,
        game_exe: beside_me("ptv-bench-game.exe")?,
        playerstv_exe: if recorder == Recorder::Playerstv { beside_me("playerstv.exe")? } else { PathBuf::new() },
        ffmpeg: ffmpeg(),
        out_dir: out.to_owned(),
        clips_dir: c.clips_dir_for(recorder)?,
        hotkey: c.hotkey_for(recorder)?,
        interactive: !c.yes,
        environment,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn per_recorder_values() {
        let e = ["D:\\all".to_owned(), "obs=D:\\obs".to_owned()];
        assert_eq!(per_recorder(&e, Recorder::Obs).unwrap().as_deref(), Some("D:\\obs"));
        assert_eq!(per_recorder(&e, Recorder::Medal).unwrap().as_deref(), Some("D:\\all"));
        assert!(per_recorder(&["nope=x".to_owned()], Recorder::Obs).is_err());
    }

    #[test]
    fn shuffle_is_a_fixed_permutation() {
        let mut a = Recorder::ALL;
        shuffle(&mut a, 3);
        let mut b = Recorder::ALL;
        shuffle(&mut b, 3);
        assert_eq!(a, b);
        let mut sorted = a;
        sorted.sort();
        assert_eq!(sorted, Recorder::ALL);
    }
}
