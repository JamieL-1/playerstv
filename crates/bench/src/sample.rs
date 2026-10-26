//! Once a second, what a recorder's processes cost, and what the whole
//! system is doing. Whole-system numbers matter because some recorders do
//! their work in drivers or services that aren't their own processes; the
//! report compares them against a baseline run with no recorder.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use anyhow::Result;
use crossbeam_channel::{bounded, Sender};
use serde::{Deserialize, Serialize};

use crate::win::pdh::{Counter, Query};
use crate::win::procs::{self, ProcessCounters};

const INTERVAL: Duration = Duration::from_secs(1);
const MB: f64 = 1024.0 * 1024.0;

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct Sample {
    /// Seconds since sampling started.
    pub t_s: f64,
    /// How many processes the recorder had.
    pub processes: usize,
    pub cpu_cores: f64,
    pub private_mb: f64,
    pub working_set_mb: f64,
    pub disk_write_mb_s: f64,
    pub gpu_3d_pct: f64,
    /// Summed over the GPU's encode engines.
    pub gpu_encode_pct: f64,
    pub vram_mb: f64,
    pub sys_cpu_cores: f64,
    pub sys_gpu_3d_pct: f64,
    pub sys_gpu_encode_pct: f64,
    pub sys_vram_mb: f64,
    pub sys_disk_write_mb_s: f64,
}

pub struct Sampler {
    stop: Sender<()>,
    thread: std::thread::JoinHandle<Result<Vec<Sample>>>,
}

impl Sampler {
    /// Samples the processes matching `patterns` (see [`procs::tree`]) and
    /// the system until [`Sampler::finish`].
    pub fn start(patterns: Vec<String>) -> Result<Self> {
        let (stop, stopped) = bounded::<()>(1);
        let thread = std::thread::Builder::new().name("sampler".into()).spawn(move || {
            let patterns: Vec<&str> = patterns.iter().map(String::as_str).collect();
            let mut state = State::new()?;
            let started = Instant::now();
            let mut samples = Vec::with_capacity(600);
            state.tick(&patterns, started)?; // first readings only set the baseline
            while stopped.recv_timeout(INTERVAL).is_err() {
                samples.push(state.tick(&patterns, started)?);
            }
            Ok(samples)
        })?;
        Ok(Self { stop, thread })
    }

    pub fn finish(self) -> Result<Vec<Sample>> {
        let _ = self.stop.send(());
        self.thread.join().map_err(|_| anyhow::anyhow!("sampler thread panicked"))?
    }
}

struct State {
    query: Query,
    gpu_engine: Counter,
    gpu_memory: Counter,
    adapter_memory: Counter,
    disk_write: Counter,
    last_at: Instant,
    last_procs: HashMap<u32, ProcessCounters>,
    last_sys: (u64, u64),
}

impl State {
    fn new() -> Result<Self> {
        let query = Query::open()?;
        let gpu_engine = query.add(r"\GPU Engine(*)\Utilization Percentage")?;
        let gpu_memory = query.add(r"\GPU Process Memory(*)\Dedicated Usage")?;
        let adapter_memory = query.add(r"\GPU Adapter Memory(*)\Dedicated Usage")?;
        let disk_write = query.add(r"\PhysicalDisk(_Total)\Disk Write Bytes/sec")?;
        query.collect()?;
        Ok(Self {
            query,
            gpu_engine,
            gpu_memory,
            adapter_memory,
            disk_write,
            last_at: Instant::now(),
            last_procs: HashMap::new(),
            last_sys: procs::system_cpu()?,
        })
    }

    fn tick(&mut self, patterns: &[&str], started: Instant) -> Result<Sample> {
        let now = Instant::now();
        let secs = (now - self.last_at).as_secs_f64().max(1e-3);
        self.last_at = now;
        self.query.collect()?;

        let pids = if patterns.is_empty() { Vec::new() } else { procs::tree(&procs::list()?, patterns) };
        let mut s = Sample { t_s: (now - started).as_secs_f64(), processes: pids.len(), ..Default::default() };
        let mut current = HashMap::with_capacity(pids.len());
        for &pid in &pids {
            let Some(c) = procs::counters(pid) else { continue };
            // A process seen for the first time counts from now on.
            if let Some(prev) = self.last_procs.get(&pid) {
                s.cpu_cores += c.cpu_100ns.saturating_sub(prev.cpu_100ns) as f64 / 1e7 / secs;
                s.disk_write_mb_s += c.write_bytes.saturating_sub(prev.write_bytes) as f64 / MB / secs;
            }
            s.private_mb += c.private_bytes as f64 / MB;
            s.working_set_mb += c.working_set_bytes as f64 / MB;
            current.insert(pid, c);
        }
        self.last_procs = current;

        let sys = procs::system_cpu()?;
        let (busy, total) = (sys.0 - self.last_sys.0, sys.1 - self.last_sys.1);
        self.last_sys = sys;
        s.sys_cpu_cores = if total == 0 { 0.0 } else { busy as f64 / total as f64 * procs::logical_cpus() as f64 };

        let gpu = gpu_totals(&self.query.values(self.gpu_engine)?, &pids);
        (s.gpu_3d_pct, s.gpu_encode_pct, s.sys_gpu_3d_pct, s.sys_gpu_encode_pct) =
            (gpu.procs_3d, gpu.procs_encode, gpu.sys_3d, gpu.sys_encode);
        let sys_vram = self.query.sum(self.adapter_memory)?;
        s.vram_mb = process_memory(&self.query.values(self.gpu_memory)?, &pids, sys_vram) / MB;
        s.sys_vram_mb = sys_vram / MB;
        s.sys_disk_write_mb_s = self.query.sum(self.disk_write)? / MB;
        Ok(s)
    }
}

/// The pid and engine type in a `\GPU Engine` instance name, e.g.
/// `pid_1234_luid_0x0_0xD1A4_phys_0_eng_3_engtype_VideoEncode`.
fn parse_engine(name: &str) -> Option<(u32, &str)> {
    let pid = parse_pid(name)?;
    Some((pid, name.rsplit_once("_engtype_")?.1))
}

fn parse_pid(name: &str) -> Option<u32> {
    name.strip_prefix("pid_")?.split('_').next()?.parse().ok()
}

#[derive(Debug, Default, PartialEq)]
struct GpuTotals {
    procs_3d: f64,
    procs_encode: f64,
    sys_3d: f64,
    sys_encode: f64,
}

fn gpu_totals(values: &[(String, f64)], pids: &[u32]) -> GpuTotals {
    let mut t = GpuTotals::default();
    for (name, v) in values {
        let Some((pid, kind)) = parse_engine(name) else { continue };
        let ours = pids.contains(&pid);
        match kind {
            "3D" => {
                t.sys_3d += v;
                if ours {
                    t.procs_3d += v;
                }
            }
            "VideoEncode" => {
                t.sys_encode += v;
                if ours {
                    t.procs_encode += v;
                }
            }
            _ => {}
        }
    }
    t
}

/// Dedicated GPU memory of `pids`, in bytes. Some processes report
/// impossible values (Discord Clips claims terabytes), so a value larger
/// than the whole adapters' usage is ignored.
fn process_memory(values: &[(String, f64)], pids: &[u32], adapters_total: f64) -> f64 {
    values
        .iter()
        .filter(|(name, v)| *v <= adapters_total && parse_pid(name).is_some_and(|pid| pids.contains(&pid)))
        .map(|(_, v)| v)
        .sum()
}

/// Each field's median over a run's samples: one robust number per metric.
pub fn medians(samples: &[Sample]) -> Sample {
    let m = |f: fn(&Sample) -> f64| crate::stats::median(&samples.iter().map(f).collect::<Vec<_>>()).unwrap_or(0.0);
    Sample {
        t_s: m(|s| s.t_s),
        processes: m(|s| s.processes as f64).round() as usize,
        cpu_cores: m(|s| s.cpu_cores),
        private_mb: m(|s| s.private_mb),
        working_set_mb: m(|s| s.working_set_mb),
        disk_write_mb_s: m(|s| s.disk_write_mb_s),
        gpu_3d_pct: m(|s| s.gpu_3d_pct),
        gpu_encode_pct: m(|s| s.gpu_encode_pct),
        vram_mb: m(|s| s.vram_mb),
        sys_cpu_cores: m(|s| s.sys_cpu_cores),
        sys_gpu_3d_pct: m(|s| s.sys_gpu_3d_pct),
        sys_gpu_encode_pct: m(|s| s.sys_gpu_encode_pct),
        sys_vram_mb: m(|s| s.sys_vram_mb),
        sys_disk_write_mb_s: m(|s| s.sys_disk_write_mb_s),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_gpu_engines_by_process_and_type() {
        let v = |n: &str, x: f64| (n.to_owned(), x);
        let values = [
            v("pid_10_luid_0x00000000_0x0000D1A4_phys_0_eng_0_engtype_3D", 4.0),
            v("pid_10_luid_0x00000000_0x0000D1A4_phys_0_eng_9_engtype_VideoEncode", 11.0),
            v("pid_11_luid_0x00000000_0x0000D1A4_phys_0_eng_10_engtype_VideoEncode", 2.0),
            v("pid_99_luid_0x00000000_0x0000D1A4_phys_0_eng_0_engtype_3D", 30.0),
            v("pid_99_luid_0x00000000_0x0000D1A4_phys_0_eng_5_engtype_Copy", 50.0),
        ];
        let t = gpu_totals(&values, &[10, 11]);
        assert_eq!(t, GpuTotals { procs_3d: 4.0, procs_encode: 13.0, sys_3d: 34.0, sys_encode: 13.0 });
        let mem = [
            v("pid_10_luid_0x00000000_0x0000D1A4_phys_0", 100.0),
            v("pid_11_luid_0x00000000_0x0000D1A4_phys_0", 2.4e12), // bogus
            v("pid_99_luid_0x0_0x1_phys_0", 50.0),
        ];
        assert_eq!(process_memory(&mem, &[10, 11], 1000.0), 100.0);
    }

    #[test]
    fn samples_this_machine() {
        let sampler = Sampler::start(vec!["explorer.exe".into()]).unwrap();
        // Opening the PDH query takes a moment before the first 1 s interval.
        std::thread::sleep(Duration::from_millis(2500));
        let samples = sampler.finish().unwrap();
        let s = samples.first().expect("one sample after a second");
        assert!(s.sys_cpu_cores >= 0.0 && s.sys_cpu_cores <= procs::logical_cpus() as f64);
    }
}
