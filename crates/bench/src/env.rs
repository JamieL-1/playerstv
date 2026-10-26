//! The machine a run happened on, saved with the results so numbers from
//! different machines or drivers are never mixed up.

use std::process::Command;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Environment {
    pub os: String,
    pub cpu: String,
    pub logical_cpus: usize,
    pub ram_gb: f64,
    pub gpu: String,
    pub gpu_driver: String,
    pub display: String,
    pub git_commit: String,
    pub harness_version: String,
}

/// Best effort: anything that can't be read stays empty.
pub fn collect() -> Environment {
    #[derive(Deserialize, Default)]
    #[serde(rename_all = "PascalCase", default)]
    struct Cim {
        os: String,
        cpu: String,
        ram: f64,
        display: String,
    }
    let script = "$o=Get-CimInstance Win32_OperatingSystem; $p=Get-CimInstance Win32_Processor | Select-Object -First 1; \
        $c=Get-CimInstance Win32_ComputerSystem; \
        $v=Get-CimInstance Win32_VideoController | Where-Object CurrentRefreshRate | Select-Object -First 1; \
        [pscustomobject]@{Os=\"$($o.Caption) $($o.Version)\"; Cpu=$p.Name.Trim(); Ram=[double]$c.TotalPhysicalMemory; \
        Display=\"$($v.CurrentHorizontalResolution)x$($v.CurrentVerticalResolution) $($v.CurrentRefreshRate) Hz\"} | ConvertTo-Json -Compress";
    let cim: Cim = run("powershell", &["-NoProfile", "-Command", script])
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();

    let nvidia = run("nvidia-smi", &["--query-gpu=name,driver_version", "--format=csv,noheader"]).unwrap_or_default();
    let (gpu, driver) = nvidia.lines().next().and_then(|l| l.split_once(", ")).unwrap_or(("", ""));

    Environment {
        os: cim.os,
        cpu: cim.cpu,
        logical_cpus: crate::win::procs::logical_cpus(),
        ram_gb: (cim.ram / 1e9 * 10.0).round() / 10.0,
        gpu: gpu.trim().to_owned(),
        gpu_driver: driver.trim().to_owned(),
        display: cim.display,
        git_commit: run("git", &["rev-parse", "--short", "HEAD"]).unwrap_or_default().trim().to_owned(),
        harness_version: env!("CARGO_PKG_VERSION").to_owned(),
    }
}

fn run(program: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(program).args(args).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}
