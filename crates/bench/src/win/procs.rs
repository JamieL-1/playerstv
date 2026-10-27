//! Process listing and per-process counters: CPU time, memory, disk writes.

use std::collections::HashMap;

use anyhow::{Context, Result};
use windows::Win32::Foundation::{CloseHandle, FILETIME, HANDLE};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::ProcessStatus::{
    GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX,
};
use windows::Win32::System::Threading::{
    GetProcessIoCounters, GetProcessTimes, GetSystemTimes, OpenProcess, IO_COUNTERS, PROCESS_QUERY_LIMITED_INFORMATION,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessEntry {
    pub pid: u32,
    pub parent: u32,
    /// Lowercase executable file name, e.g. `obs64.exe`.
    pub exe: String,
}

/// Every running process.
pub fn list() -> Result<Vec<ProcessEntry>> {
    // SAFETY: plain FFI call; the snapshot handle is closed below.
    let snap = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0).context("CreateToolhelp32Snapshot")? };
    let snap = OwnedHandle(snap);
    let mut entry = PROCESSENTRY32W { dwSize: size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
    let mut out = Vec::with_capacity(512);
    // SAFETY: `entry` has dwSize set and is a valid out-pointer; the
    // snapshot handle is open.
    let mut more = unsafe { Process32FirstW(snap.0, &mut entry) }.is_ok();
    while more {
        let len = entry.szExeFile.iter().position(|&c| c == 0).unwrap_or(entry.szExeFile.len());
        out.push(ProcessEntry {
            pid: entry.th32ProcessID,
            parent: entry.th32ParentProcessID,
            exe: String::from_utf16_lossy(&entry.szExeFile[..len]).to_lowercase(),
        });
        // SAFETY: as above.
        more = unsafe { Process32NextW(snap.0, &mut entry) }.is_ok();
    }
    Ok(out)
}

/// Cumulative counters for one process.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ProcessCounters {
    /// Kernel + user CPU time, in 100 ns units.
    pub cpu_100ns: u64,
    pub private_bytes: u64,
    pub working_set_bytes: u64,
    pub write_bytes: u64,
}

/// `None` if the process is gone or we may not query it.
pub fn counters(pid: u32) -> Option<ProcessCounters> {
    // SAFETY: plain FFI call; a returned handle is closed by OwnedHandle.
    let process = OwnedHandle(unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }.ok()?);
    let (mut created, mut exited, mut kernel, mut user) = Default::default();
    let mut mem =
        PROCESS_MEMORY_COUNTERS_EX { cb: size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32, ..Default::default() };
    let mut io = IO_COUNTERS::default();
    // SAFETY: the handle is open with query rights; every out-pointer is a
    // valid local, and `mem` is passed with its real size (the EX struct
    // starts with the plain one).
    unsafe {
        GetProcessTimes(process.0, &mut created, &mut exited, &mut kernel, &mut user).ok()?;
        GetProcessMemoryInfo(process.0, (&raw mut mem).cast::<PROCESS_MEMORY_COUNTERS>(), mem.cb).ok()?;
        // Not every process allows I/O counters; report zero writes then.
        let _ = GetProcessIoCounters(process.0, &mut io);
    }
    Some(ProcessCounters {
        cpu_100ns: ft(kernel) + ft(user),
        private_bytes: mem.PrivateUsage as u64,
        working_set_bytes: mem.WorkingSetSize as u64,
        write_bytes: io.WriteTransferCount,
    })
}

/// Cumulative system CPU times in 100 ns units: (busy, total) summed over
/// all logical processors.
pub fn system_cpu() -> Result<(u64, u64)> {
    let (mut idle, mut kernel, mut user) = (FILETIME::default(), FILETIME::default(), FILETIME::default());
    // SAFETY: valid out-pointers to locals.
    unsafe { GetSystemTimes(Some(&mut idle), Some(&mut kernel), Some(&mut user)).context("GetSystemTimes")? };
    // Kernel time includes idle time.
    let total = ft(kernel) + ft(user);
    Ok((total - ft(idle), total))
}

pub fn logical_cpus() -> usize {
    std::thread::available_parallelism().map_or(1, |n| n.get())
}

fn ft(t: FILETIME) -> u64 {
    (t.dwHighDateTime as u64) << 32 | t.dwLowDateTime as u64
}

/// The processes whose exe matches one of `patterns`, plus all their
/// descendants (e.g. the ffmpeg playerstv starts). A pattern ending in `*`
/// matches by prefix; matching ignores case.
pub fn tree(all: &[ProcessEntry], patterns: &[&str]) -> Vec<u32> {
    let matches = |exe: &str| {
        patterns.iter().any(|p| match p.strip_suffix('*') {
            Some(prefix) => exe.starts_with(&prefix.to_lowercase()),
            None => exe == p.to_lowercase(),
        })
    };
    let mut children: HashMap<u32, Vec<u32>> = HashMap::new();
    for p in all {
        children.entry(p.parent).or_default().push(p.pid);
    }
    let mut out: Vec<u32> = all.iter().filter(|p| matches(&p.exe)).map(|p| p.pid).collect();
    let mut next = 0;
    while next < out.len() {
        let pid = out[next];
        for &child in children.get(&pid).into_iter().flatten() {
            // Pid 0's "children" are bogus; also guard against pid reuse loops.
            if pid != 0 && !out.contains(&child) {
                out.push(child);
            }
        }
        next += 1;
    }
    out
}

struct OwnedHandle(HANDLE);

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        // SAFETY: we own this handle and close it once.
        let _ = unsafe { CloseHandle(self.0) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(pid: u32, parent: u32, exe: &str) -> ProcessEntry {
        ProcessEntry { pid, parent, exe: exe.into() }
    }

    #[test]
    fn tree_takes_matches_and_descendants() {
        let all = [
            p(1, 0, "explorer.exe"),
            p(10, 1, "playerstv.exe"),
            p(11, 10, "ffmpeg.exe"),
            p(12, 11, "conhost.exe"),
            p(20, 1, "ffmpeg.exe"), // someone else's ffmpeg
            p(30, 1, "medal.exe"),
            p(31, 1, "medalencoder.exe"),
        ];
        let mut t = tree(&all, &["PlayersTV.exe"]);
        t.sort();
        assert_eq!(t, [10, 11, 12]);
        let mut t = tree(&all, &["medal*"]);
        t.sort();
        assert_eq!(t, [30, 31]);
    }

    #[test]
    fn reads_our_own_counters() {
        let c = counters(std::process::id()).unwrap();
        assert!(c.private_bytes > 0 && c.working_set_bytes > 0);
        let (busy, total) = system_cpu().unwrap();
        assert!(busy <= total);
    }
}
