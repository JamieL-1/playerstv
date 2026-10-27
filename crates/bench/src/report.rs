//! Turns a folder of runs into the comparison: each metric per recorder
//! with a 95% interval, playerstv's verdict against each competitor, and
//! which runs failed the correctness checks.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::recorders::Recorder;
use crate::run::{IdleResult, RunResult};
use crate::stats::{self, Estimate};

/// A/V sync a clip must meet for its run to count; see BENCHMARKS.md.
/// Dropped frames don't exclude a run: they're compared as a metric.
pub const MAX_AV_OFFSET_MS: f64 = 45.0;

/// Share of the frames the game drew in the clip's span that are missing
/// (or, with `repeats`, that are a repeat of the previous frame), in %.
fn frame_loss_pct(r: &RunResult, repeats: bool) -> Option<f64> {
    let c = r.clip.as_ref()?;
    let drawn = (c.last_index? - c.first_index? + 1).saturating_sub(c.skipped_by_game);
    let lost = if repeats { c.duplicates } else { c.dropped };
    (drawn > 0).then(|| lost as f64 / drawn as f64 * 100.0)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Better {
    Lower,
    Higher,
}

pub struct Metric {
    pub key: &'static str,
    pub label: &'static str,
    pub better: Better,
    /// The value for one run, given the baseline run of the same round.
    value: fn(&RunResult, Option<&RunResult>) -> Option<f64>,
}

fn delta(r: Option<f64>, b: Option<f64>) -> Option<f64> {
    Some(r? - b?)
}

pub const METRICS: &[Metric] = &[
    Metric {
        key: "sys_cpu_cores",
        label: "Whole-system CPU, cores above baseline",
        better: Better::Lower,
        value: |r, b| delta(Some(r.metrics.sys_cpu_cores), b.map(|b| b.metrics.sys_cpu_cores)),
    },
    Metric {
        key: "cpu_cores",
        label: "Recorder processes CPU, cores",
        better: Better::Lower,
        value: |r, _| Some(r.metrics.cpu_cores),
    },
    Metric {
        key: "sys_gpu_3d_pct",
        label: "GPU 3D engine, % above baseline",
        better: Better::Lower,
        value: |r, b| delta(Some(r.metrics.sys_gpu_3d_pct), b.map(|b| b.metrics.sys_gpu_3d_pct)),
    },
    Metric {
        key: "sys_gpu_encode_pct",
        label: "GPU video encode, % above baseline",
        better: Better::Lower,
        value: |r, b| delta(Some(r.metrics.sys_gpu_encode_pct), b.map(|b| b.metrics.sys_gpu_encode_pct)),
    },
    Metric {
        key: "sys_vram_mb",
        label: "GPU memory, MB above baseline",
        better: Better::Lower,
        value: |r, b| delta(Some(r.metrics.sys_vram_mb), b.map(|b| b.metrics.sys_vram_mb)),
    },
    Metric {
        key: "private_mb",
        label: "Recorder processes RAM (private), MB",
        better: Better::Lower,
        value: |r, _| Some(r.metrics.private_mb),
    },
    Metric {
        key: "sys_disk_write_mb_s",
        label: "Disk writes, MB/s above baseline",
        better: Better::Lower,
        value: |r, b| delta(Some(r.metrics.sys_disk_write_mb_s), b.map(|b| b.metrics.sys_disk_write_mb_s)),
    },
    Metric {
        key: "frame_p99_ms",
        label: "Game frame time p99, ms above baseline",
        better: Better::Lower,
        value: |r, b| delta(r.frame_interval_ms.map(|s| s.p99), b.and_then(|b| b.frame_interval_ms).map(|s| s.p99)),
    },
    Metric {
        key: "frame_p999_ms",
        label: "Game frame time p99.9, ms above baseline",
        better: Better::Lower,
        value: |r, b| delta(r.frame_interval_ms.map(|s| s.p999), b.and_then(|b| b.frame_interval_ms).map(|s| s.p999)),
    },
    Metric { key: "save_ms", label: "Save time, ms", better: Better::Lower, value: |r, _| r.save_ms },
    Metric {
        key: "dropped_pct",
        label: "Clip frames dropped, % of frames drawn",
        better: Better::Lower,
        value: |r, _| frame_loss_pct(r, false),
    },
    Metric {
        key: "repeated_pct",
        label: "Clip frames repeated, % of frames drawn",
        better: Better::Lower,
        value: |r, _| frame_loss_pct(r, true),
    },
    Metric { key: "ssim", label: "Clip quality, SSIM", better: Better::Higher, value: |r, _| r.clip.as_ref()?.ssim },
    Metric {
        key: "bitrate_mbps",
        label: "Clip bitrate, Mbps",
        better: Better::Lower,
        value: |r, _| Some(r.clip.as_ref()?.bitrate_mbps),
    },
];

/// Why a run's clip doesn't count; empty if it passes.
pub fn gate_failures(r: &RunResult) -> Vec<String> {
    if r.recorder == Recorder::None {
        return Vec::new();
    }
    let Some(c) = &r.clip else { return vec!["no clip was saved or it couldn't be read".into()] };
    let mut out = Vec::new();
    if c.unreadable > 0 {
        out.push(format!("{} frames too damaged to identify", c.unreadable));
    }
    let expected = r.settings.buffer_seconds as f64 - 1.0 / r.settings.fps as f64;
    if c.duration_s < expected {
        out.push(format!("clip is {:.2} s, shorter than the {} s buffer", c.duration_s, r.settings.buffer_seconds));
    }
    if r.settings.audio {
        if c.audio_tracks == 0 {
            out.push("no audio track".into());
        }
        match c.av_offset_ms {
            Some(ms) if ms.abs() > MAX_AV_OFFSET_MS => out.push(format!("audio is {ms:+.0} ms off the picture")),
            None if c.audio_tracks > 0 => out.push("couldn't measure A/V sync".into()),
            _ => {}
        }
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Verdict {
    /// playerstv is better beyond noise.
    Better,
    Worse,
    NoDifference,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricResult {
    pub key: String,
    pub label: String,
    pub by_recorder: BTreeMap<Recorder, Estimate>,
    /// playerstv minus each competitor, with its verdict.
    pub vs_playerstv: BTreeMap<Recorder, (Estimate, Verdict)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Report {
    pub runs: usize,
    pub rounds: Vec<u32>,
    pub recorders: Vec<Recorder>,
    pub metrics: Vec<MetricResult>,
    /// Runs left out because their clip failed a check.
    pub excluded: Vec<(Recorder, u32, Vec<String>)>,
    pub warnings: Vec<String>,
    pub idle: Vec<IdleResult>,
    pub first_run: Option<RunResult>,
}

pub fn load(dir: &Path) -> Result<(Vec<RunResult>, Vec<IdleResult>)> {
    let (mut runs, mut idle) = (Vec::new(), Vec::new());
    for entry in std::fs::read_dir(dir).with_context(|| dir.display().to_string())? {
        let path = entry?.path();
        if path.extension().is_none_or(|e| e != "json") || path.file_name().is_some_and(|n| n == "report.json") {
            continue;
        }
        let text = std::fs::read_to_string(&path)?;
        if path.file_name().is_some_and(|n| n.to_string_lossy().starts_with("idle-")) {
            idle.push(serde_json::from_str(&text).with_context(|| path.display().to_string())?);
        } else {
            runs.push(serde_json::from_str(&text).with_context(|| path.display().to_string())?);
        }
    }
    runs.sort_by_key(|r: &RunResult| (r.round, r.recorder));
    idle.sort_by_key(|r: &IdleResult| r.recorder);
    Ok((runs, idle))
}

/// Fails if the runs weren't all made with the same settings on the same
/// machine. Baselines are matched by round and the report describes every
/// run with the first one's setup, so a mix would compare unlike numbers.
pub fn check_consistent(runs: &[RunResult], idle: &[IdleResult]) -> Result<()> {
    let setup = |r: &RunResult| (r.settings, r.gpu_load, r.warmup_s, r.measure_s);
    let mut problems = Vec::new();
    if let Some(first) = runs.first() {
        for r in runs.iter().filter(|r| setup(r) != setup(first)) {
            problems.push(format!(
                "{} round {} used different settings from {} round {}",
                r.recorder.label(),
                r.round,
                first.recorder.label(),
                first.round
            ));
        }
    }
    let machines = runs
        .iter()
        .map(|r| (&r.environment, format!("{} round {}", r.recorder.label(), r.round)))
        .chain(idle.iter().map(|r| (&r.environment, format!("{} idle", r.recorder.label()))));
    let mut first_machine = None;
    for (env, name) in machines {
        if *first_machine.get_or_insert(env) != env {
            problems.push(format!("{name} ran on a different machine, driver or build"));
        }
    }
    if !problems.is_empty() {
        bail!("these results can't be compared:\n  {}", problems.join("\n  "));
    }
    Ok(())
}

pub fn build(runs: &[RunResult], idle: Vec<IdleResult>) -> Report {
    let baseline = |round: u32| runs.iter().find(|r| r.round == round && r.recorder == Recorder::None);
    let mut excluded = Vec::new();
    let mut counted: Vec<&RunResult> = Vec::new();
    for r in runs {
        let failures = gate_failures(r);
        if failures.is_empty() {
            counted.push(r);
        } else {
            excluded.push((r.recorder, r.round, failures));
        }
    }

    let mut recorders: Vec<Recorder> = runs.iter().map(|r| r.recorder).collect();
    recorders.sort();
    recorders.dedup();
    let mut rounds: Vec<u32> = runs.iter().map(|r| r.round).collect();
    rounds.sort();
    rounds.dedup();

    let metrics = METRICS
        .iter()
        .map(|m| {
            let values = |rec: Recorder| -> Vec<f64> {
                counted.iter().filter(|r| r.recorder == rec).filter_map(|r| (m.value)(r, baseline(r.round))).collect()
            };
            let by_recorder: BTreeMap<Recorder, Estimate> =
                recorders.iter().filter_map(|&rec| Some((rec, stats::median_ci(&values(rec))?))).collect();
            let ours = values(Recorder::Playerstv);
            let vs_playerstv = recorders
                .iter()
                .filter(|&&rec| rec != Recorder::Playerstv && rec != Recorder::None)
                .filter_map(|&rec| {
                    let d = stats::diff_ci(&ours, &values(rec))?;
                    let verdict = match m.better {
                        Better::Lower if d.hi < 0.0 => Verdict::Better,
                        Better::Lower if d.lo > 0.0 => Verdict::Worse,
                        Better::Higher if d.lo > 0.0 => Verdict::Better,
                        Better::Higher if d.hi < 0.0 => Verdict::Worse,
                        _ => Verdict::NoDifference,
                    };
                    Some((rec, (d, verdict)))
                })
                .collect();
            MetricResult { key: m.key.into(), label: m.label.into(), by_recorder, vs_playerstv }
        })
        .collect();

    let mut warnings: Vec<String> = runs
        .iter()
        .flat_map(|r| r.warnings.iter().map(move |w| format!("{} round {}: {w}", r.recorder.label(), r.round)))
        .collect();
    for &round in &rounds {
        if baseline(round).is_none() {
            warnings.push(format!("round {round} has no baseline run, so its above-baseline numbers are missing"));
        }
    }

    Report { runs: runs.len(), rounds, recorders, metrics, excluded, warnings, idle, first_run: runs.first().cloned() }
}

fn fmt_estimate(e: &Estimate) -> String {
    let digits = if e.median.abs() >= 100.0 {
        0
    } else if e.median.abs() >= 1.0 {
        2
    } else {
        3
    };
    format!("{:.*} ({:.*} to {:.*})", digits, e.median, digits, e.lo, digits, e.hi)
}

pub fn markdown(report: &Report) -> String {
    let mut md = String::new();
    let _ = writeln!(md, "# Recorder benchmark\n");
    if let Some(r) = &report.first_run {
        let s = &r.settings;
        let e = &r.environment;
        let _ = writeln!(
            md,
            "{} rounds. The test game drew {}x{} at {} fps. Each recorder had a {} s buffer, \
             {}, and was measured for {:.0} s after a {:.0} s warm-up.\n",
            report.rounds.len(),
            s.width,
            s.height,
            s.fps,
            s.buffer_seconds,
            match (s.audio, s.mic) {
                (true, true) => "with game audio and the microphone mixed",
                (true, false) => "with game audio",
                (false, true) => "with the microphone",
                (false, false) => "without audio",
            },
            r.measure_s,
            r.warmup_s
        );
        let _ = writeln!(
            md,
            "Machine: {}, {} ({} threads), {} GB RAM, {} (driver {}), {}. {}. Commit {}.\n",
            e.cpu,
            e.os,
            e.logical_cpus,
            e.ram_gb,
            e.gpu,
            e.gpu_driver,
            e.display,
            if r.gpu_load > 0 { format!("Extra GPU load {}", r.gpu_load) } else { "No extra GPU load".into() },
            e.git_commit
        );
    }
    let _ = writeln!(
        md,
        "Each cell is the median over rounds with its 95% bootstrap interval. \
         \"Above baseline\" subtracts the same round's run with no recorder.\n"
    );

    let cols: Vec<Recorder> = report.recorders.clone();
    let _ = write!(md, "| Metric |");
    for r in &cols {
        let _ = write!(md, " {} |", r.label());
    }
    let _ = write!(md, "\n|---|");
    for _ in &cols {
        let _ = write!(md, "---|");
    }
    let _ = writeln!(md);
    for m in &report.metrics {
        let _ = write!(md, "| {} |", m.label);
        for r in &cols {
            let cell = m.by_recorder.get(r).map(fmt_estimate).unwrap_or_else(|| "n/a".into());
            let _ = write!(md, " {cell} |");
        }
        let _ = writeln!(md);
    }

    let competitors: Vec<Recorder> =
        cols.iter().copied().filter(|&r| r != Recorder::None && r != Recorder::Playerstv).collect();
    if !competitors.is_empty() {
        let _ = writeln!(md, "\n## playerstv against each recorder\n");
        let _ = writeln!(
            md,
            "Better or worse only when the 95% interval of the difference excludes zero; \
             otherwise there's no measurable difference.\n"
        );
        let _ = write!(md, "| Metric |");
        for r in &competitors {
            let _ = write!(md, " vs {} |", r.label());
        }
        let _ = write!(md, "\n|---|");
        for _ in &competitors {
            let _ = write!(md, "---|");
        }
        let _ = writeln!(md);
        for m in &report.metrics {
            let _ = write!(md, "| {} |", m.label);
            for r in &competitors {
                let cell = match m.vs_playerstv.get(r) {
                    Some((d, Verdict::Better)) => format!("better, {}", fmt_estimate(d)),
                    Some((d, Verdict::Worse)) => format!("worse, {}", fmt_estimate(d)),
                    Some((d, Verdict::NoDifference)) => format!("no difference, {}", fmt_estimate(d)),
                    None => "n/a".into(),
                };
                let _ = write!(md, " {cell} |");
            }
            let _ = writeln!(md);
        }
    }

    if !report.idle.is_empty() {
        let _ = writeln!(md, "\n## Idle, no game running\n");
        let _ = writeln!(md, "| Recorder | CPU, cores | RAM (private), MB | Processes |\n|---|---|---|---|");
        for i in &report.idle {
            let _ = writeln!(
                md,
                "| {} | {:.3} | {:.1} | {} |",
                i.recorder.label(),
                i.metrics.cpu_cores,
                i.metrics.private_mb,
                i.metrics.processes
            );
        }
    }

    if !report.excluded.is_empty() {
        let _ = writeln!(md, "\n## Runs left out\n");
        for (rec, round, why) in &report.excluded {
            let _ = writeln!(md, "- {} round {round}: {}", rec.label(), why.join("; "));
        }
    }
    if !report.warnings.is_empty() {
        let _ = writeln!(md, "\n## Warnings\n");
        for w in &report.warnings {
            let _ = writeln!(md, "- {w}");
        }
    }
    md
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analyze::ClipReport;
    use crate::recorders::Settings;
    use crate::sample::Sample;

    fn run(recorder: Recorder, round: u32, sys_cpu: f64, dropped: u64) -> RunResult {
        RunResult {
            recorder,
            round,
            started: String::new(),
            settings: Settings {
                width: 1920,
                height: 1080,
                fps: 60,
                buffer_seconds: 30,
                bitrate_mbps: 14,
                audio: false,
                mic: false,
            },
            gpu_load: 0,
            warmup_s: 30.0,
            measure_s: 180.0,
            environment: Default::default(),
            checklist: Vec::new(),
            warnings: Vec::new(),
            metrics: Sample { sys_cpu_cores: sys_cpu, ..Default::default() },
            samples: Vec::new(),
            frame_interval_ms: None,
            game_work_ms: None,
            save_ms: Some(500.0),
            clip: (recorder != Recorder::None).then(|| ClipReport {
                path: "c.mp4".into(),
                container: "mp4".into(),
                video_codec: "h264".into(),
                width: 1920,
                height: 1080,
                audio_tracks: 0,
                duration_s: 30.0,
                size_bytes: 1,
                bitrate_mbps: 14.0,
                frames: 1800,
                unreadable: 0,
                first_index: Some(0),
                last_index: Some(1799),
                dropped,
                duplicates: 0,
                skipped_by_game: 0,
                longest_gap: dropped.min(1),
                ssim: Some(0.99),
                psnr_db: Some(40.0),
                av_offset_ms: None,
            }),
        }
    }

    #[test]
    fn compares_against_baseline_and_excludes_bad_clips() {
        let mut runs = Vec::new();
        for round in 1..=5 {
            let jitter = round as f64 * 0.01;
            runs.push(run(Recorder::None, round, 1.0 + jitter, 0));
            runs.push(run(Recorder::Playerstv, round, 1.05 + jitter, 0));
            let mut obs = run(Recorder::Obs, round, 1.5 + jitter, 18 + round as u64);
            if round == 3 {
                // A clip shorter than the buffer doesn't count.
                obs.clip.as_mut().unwrap().duration_s = 20.0;
            }
            runs.push(obs);
        }
        let report = build(&runs, Vec::new());
        assert_eq!(report.excluded.len(), 1);
        assert_eq!(report.excluded[0].0, Recorder::Obs);

        // Dropped frames are compared, not excluded: 1% and up for OBS.
        let dropped = report.metrics.iter().find(|m| m.key == "dropped_pct").unwrap();
        assert!(dropped.by_recorder[&Recorder::Obs].median > 1.0);
        assert_eq!(dropped.vs_playerstv[&Recorder::Obs].1, Verdict::Better);

        let cpu = report.metrics.iter().find(|m| m.key == "sys_cpu_cores").unwrap();
        assert!((cpu.by_recorder[&Recorder::Playerstv].median - 0.05).abs() < 1e-9);
        assert!((cpu.by_recorder[&Recorder::Obs].median - 0.5).abs() < 1e-9);
        assert_eq!(cpu.vs_playerstv[&Recorder::Obs].1, Verdict::Better);
        let ssim = report.metrics.iter().find(|m| m.key == "ssim").unwrap();
        assert_eq!(ssim.vs_playerstv[&Recorder::Obs].1, Verdict::NoDifference);

        let md = markdown(&report);
        assert!(md.contains("| Whole-system CPU, cores above baseline |"));
        assert!(md.contains("OBS replay buffer round 3: clip is 20.00 s, shorter than the 30 s buffer"));
        assert!(!md.contains('\u{2014}'), "no em dashes in the report");
    }

    #[test]
    fn rejects_runs_from_different_setups() {
        let mut runs = vec![run(Recorder::None, 1, 1.0, 0), run(Recorder::Playerstv, 1, 1.1, 0)];
        check_consistent(&runs, &[]).unwrap();

        let mut old = run(Recorder::None, 2, 1.0, 0);
        old.settings.width = 2560;
        runs.push(old);
        let err = check_consistent(&runs, &[]).unwrap_err().to_string();
        assert!(err.contains("No recorder round 2 used different settings"), "{err}");

        runs.pop();
        let mut elsewhere = run(Recorder::Playerstv, 2, 1.1, 0);
        elsewhere.environment.gpu_driver = "999.99".into();
        runs.push(elsewhere);
        let err = check_consistent(&runs, &[]).unwrap_err().to_string();
        assert!(err.contains("playerstv round 2 ran on a different machine"), "{err}");
    }
}
