//! The test game's sound: a faint 1 kHz tone with a louder click at the
//! start of every second, timed from the same clock as the flash frames, so
//! the analyzer can measure audio/video sync in a recording. The clicks are
//! 20 to 80 ms long in turn (see [`pattern::click_code`]), so a click can't
//! be mistaken for one a second earlier or later.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result};
use ptv_bench::pattern;
use wasapi::{DeviceEnumerator, Direction, SampleType, StreamMode, WaveFormat};

pub const RATE: u32 = 48_000;
const CHANNELS: usize = 2;
const TONE_HZ: f64 = 1000.0;
// Kept quiet: the analyzer finds clicks relative to the loudest sample, so
// only the ratio between these matters.
pub const QUIET: f32 = 0.005;
pub const LOUD: f32 = 0.05;

/// One sample at position `n` (samples since `t0`).
pub fn sample(n: u64) -> f32 {
    let (second, into) = (n / RATE as u64, n % RATE as u64);
    let click = ((pattern::click_code(second) + 1) as f64 * pattern::CLICK_UNIT_S * RATE as f64) as u64;
    let amp = if into < click { LOUD } else { QUIET };
    amp * (std::f64::consts::TAU * TONE_HZ * n as f64 / RATE as f64).sin() as f32
}

pub struct Tone {
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Tone {
    /// Plays on the default output device, sample 0 lining up with `t0`.
    pub fn start(t0: Instant) -> Result<Self> {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let thread = std::thread::Builder::new().name("tone".into()).spawn(move || {
            if let Err(e) = play(t0, &flag) {
                eprintln!("audio stopped: {e:#}");
            }
        })?;
        Ok(Self { stop, thread: Some(thread) })
    }
}

impl Drop for Tone {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn play(t0: Instant, stop: &AtomicBool) -> Result<()> {
    let _ = wasapi::initialize_mta();
    let device = DeviceEnumerator::new()?.get_default_device(&Direction::Render).context("default output device")?;
    let mut client = device.get_iaudioclient()?;
    let format = WaveFormat::new(32, 32, &SampleType::Float, RATE as usize, CHANNELS, None);
    client.initialize_client(
        &format,
        &Direction::Render,
        &StreamMode::EventsShared { autoconvert: true, buffer_duration_hns: 200_000 },
    )?;
    let event = client.set_get_eventhandle()?;
    let render = client.get_audiorenderclient()?;

    // Start where the clock is now, so the clicks stay on whole seconds of
    // `t0` even if this thread started late.
    let mut n = (t0.elapsed().as_secs_f64() * RATE as f64) as u64;
    let mut buf: Vec<u8> = Vec::with_capacity(RATE as usize * CHANNELS * 4 / 10);
    let fill = |n: &mut u64, frames: usize, buf: &mut Vec<u8>| {
        buf.clear();
        for _ in 0..frames {
            let s = sample(*n).to_le_bytes();
            for _ in 0..CHANNELS {
                buf.extend_from_slice(&s);
            }
            *n += 1;
        }
    };
    let frames = client.get_available_space_in_frames()? as usize;
    fill(&mut n, frames, &mut buf);
    render.write_to_device(frames, &buf, None)?;
    client.start_stream()?;

    while !stop.load(Ordering::Relaxed) {
        if event.wait_for_event(100).is_err() {
            continue;
        }
        let frames = client.get_available_space_in_frames()? as usize;
        fill(&mut n, frames, &mut buf);
        render.write_to_device(frames, &buf, None)?;
    }
    client.stop_stream()?;
    Ok(())
}
