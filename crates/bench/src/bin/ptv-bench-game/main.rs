//! A stand-in game for benchmarks: draws `ptv_bench::pattern` frames at a
//! fixed rate, logs when every frame was presented and, with `--tone`, plays
//! a quiet clicking tone. Close it with Esc, or pass `--seconds`.

mod gpu;
mod tone;
mod window;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use clap::Parser;
use ptv_bench::frame_log::{self, FrameLog, FrameRecord};
use ptv_bench::pattern::Pattern;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::Threading::{
    CreateWaitableTimerExW, SetWaitableTimer, WaitForSingleObject, CREATE_WAITABLE_TIMER_HIGH_RESOLUTION, INFINITE,
    TIMER_ALL_ACCESS,
};

#[derive(Parser)]
#[command(about = "Test game for playerstv benchmarks")]
struct Args {
    /// Frames per second to draw.
    #[arg(long, default_value_t = 60)]
    fps: u32,
    /// A normal window with this client size (e.g. 1920x1080) instead of
    /// borderless over the whole primary monitor.
    #[arg(long, value_parser = parse_size)]
    windowed: Option<(u32, u32)>,
    /// Quit after this many seconds.
    #[arg(long)]
    seconds: Option<u64>,
    /// Extra full-frame GPU copies per frame, to load the GPU like a game.
    #[arg(long, default_value_t = 0)]
    gpu_load: u32,
    /// Vertical blanks each Present waits for: 0 presents straight away, 1
    /// is vsync.
    #[arg(long, default_value_t = 0)]
    sync_interval: u32,
    /// Play the clicking tone (off by default because it's audible). Needed
    /// to measure audio capture cost and A/V sync.
    #[arg(long)]
    tone: bool,
    /// Where to write the frame log (CSV).
    #[arg(long)]
    log: Option<PathBuf>,
}

fn parse_size(s: &str) -> Result<(u32, u32), String> {
    let (w, h) = s.split_once('x').ok_or("expected WIDTHxHEIGHT")?;
    Ok((w.parse().map_err(|_| "bad width")?, h.parse().map_err(|_| "bad height")?))
}

fn main() -> Result<()> {
    let args = Args::parse();
    let (window, width, height) = window::Window::open(args.windowed)?;
    let pattern = Pattern::new(width, height, args.fps);
    let presenter = gpu::Presenter::new(window.hwnd, width, height, args.gpu_load, args.sync_interval)?;
    let timer = Timer::new()?;
    eprintln!("drawing {width}x{height} at {} fps", args.fps);

    let period = Duration::from_secs_f64(1.0 / args.fps as f64);
    let end = args.seconds.map(Duration::from_secs);
    let mut frame = vec![0u8; pattern.frame_bytes()];
    let mut log: Vec<FrameRecord> = Vec::with_capacity(args.fps as usize * args.seconds.unwrap_or(600) as usize);
    let mut skipped = 0u64;

    let t0 = Instant::now();
    let t0_unix_us = frame_log::unix_us_now();
    let _tone = args.tone.then(|| tone::Tone::start(t0)).transpose()?;
    let mut index = 0u64;
    while window.pump() {
        if end.is_some_and(|end| t0.elapsed() >= end) {
            break;
        }
        let started = Instant::now();
        pattern.render(index, &mut frame);
        presenter.present(&frame)?;
        let presented = t0.elapsed();
        log.push(FrameRecord {
            index,
            present_us: presented.as_micros() as u64,
            work_us: started.elapsed().as_micros() as u64,
        });

        // Index n belongs to time n × period, which keeps the flash frames
        // on the same whole seconds as the audio clicks. A late frame skips
        // the indices whose time has passed; the log shows which.
        index += 1;
        let due = period * index as u32;
        if presented > due + period {
            let behind = ((presented - due).as_secs_f64() / period.as_secs_f64()) as u64;
            index += behind;
            skipped += behind;
        }
        timer.sleep_until(t0 + period * index as u32);
    }

    let elapsed = t0.elapsed().as_secs_f64();
    eprintln!("{} frames in {elapsed:.1} s, {skipped} skipped", log.len());
    if let Some(path) = &args.log {
        FrameLog { t0_unix_us, pattern, frames: log }
            .write(path)
            .with_context(|| format!("writing {}", path.display()))?;
    }
    Ok(())
}

/// A high-resolution waitable timer: wakes within about 0.5 ms of the
/// deadline, where `thread::sleep` can be off by a whole millisecond tick.
struct Timer(HANDLE);

impl Timer {
    fn new() -> Result<Self> {
        // SAFETY: plain FFI call; the handle is closed in Drop.
        let h = unsafe {
            CreateWaitableTimerExW(None, None, CREATE_WAITABLE_TIMER_HIGH_RESOLUTION, TIMER_ALL_ACCESS.0)
                .context("CreateWaitableTimerExW")?
        };
        Ok(Self(h))
    }

    fn sleep_until(&self, deadline: Instant) {
        let Some(wait) = deadline.checked_duration_since(Instant::now()) else { return };
        // Negative means relative, in 100 ns units.
        let due = -((wait.as_nanos() / 100) as i64).max(1);
        // SAFETY: `self.0` is a live timer handle we own; `due` outlives
        // SetWaitableTimer, which copies it.
        unsafe {
            if SetWaitableTimer(self.0, &due, 0, None, None, false).is_ok() {
                WaitForSingleObject(self.0, INFINITE);
            } else {
                std::thread::sleep(wait);
            }
        }
    }
}

impl Drop for Timer {
    fn drop(&mut self) {
        // SAFETY: we own the handle and close it once.
        let _ = unsafe { CloseHandle(self.0) };
    }
}
