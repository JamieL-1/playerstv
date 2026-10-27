//! WASAPI capture threads that stream 48 kHz stereo f32 PCM into named pipes.
//!
//! Each feeder runs on a wall clock that starts at an origin the recorder
//! picks before launching ffmpeg, and ffmpeg times video on the same clock.
//! Silence fills the time until the source opens (process loopback can take
//! 100 ms) and whenever WASAPI hands us nothing (common for process
//! loopback), so ffmpeg's audio timeline keeps pace with video; if data
//! arrives late after padding, the overshoot is dropped. Drift is therefore
//! bounded to a few tens of ms.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use ptv_core::ffmpeg_args::{AUDIO_CHANNELS, AUDIO_RATE};
use wasapi::{
    AudioCaptureClient, AudioClient, DeviceEnumerator, Direction, Handle, SampleType, StreamMode, WaveFormat,
};

use crate::pipe::{self, PipeServer};

const BYTES_PER_FRAME: usize = 4 * AUDIO_CHANNELS as usize;
/// Pad with silence once we're this far behind the wall clock.
const MAX_BEHIND_FRAMES: u64 = AUDIO_RATE as u64 / 50; // 20 ms
/// Drop captured audio once we're this far ahead of the wall clock.
const MAX_AHEAD_FRAMES: u64 = AUDIO_RATE as u64 / 10; // 100 ms

#[derive(Debug, Clone)]
pub enum AudioSource {
    /// Everything a process (and its children) plays.
    Process { pid: u32 },
    /// Everything playing on the default output device.
    Desktop,
    /// A capture device by friendly name, or the default mic.
    Mic { device: Option<String> },
}

/// A running pipe feeder. Stops when dropped, ffmpeg closes the pipe, or the
/// source fails.
pub struct PipeFeeder {
    pipe_name: String,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl PipeFeeder {
    /// Creates the pipe and starts the feeder thread. ffmpeg must then be
    /// started with `pipe_path()` as an input. The first sample written is
    /// the sound at `origin`.
    pub fn start(source: AudioSource, pipe_name: String, origin: Instant) -> Result<Self> {
        let server = PipeServer::create(&pipe_name)?;
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let stop = stop.clone();
            let name = pipe_name.clone();
            std::thread::Builder::new().name(format!("audio:{name}")).spawn(move || {
                if let Err(e) = run(source, &server, &stop, origin) {
                    if !stop.load(Ordering::Relaxed) {
                        tracing::warn!("audio feeder {name} ended: {e:#}");
                    }
                }
            })?
        };
        Ok(Self { pipe_name, stop, thread: Some(thread) })
    }

    pub fn pipe_path(&self) -> &str {
        &self.pipe_name
    }

    /// Marks the feeder as stopping, so ffmpeg closing the pipe isn't
    /// reported as an error.
    pub fn signal_stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

impl Drop for PipeFeeder {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        // Unblock a thread still waiting for ffmpeg to connect.
        pipe::poke(&self.pipe_name);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Friendly names of active capture (microphone) devices.
///
/// Runs on its own thread: it needs a multithreaded COM apartment, which
/// would break a GUI thread that later wants a single-threaded one.
pub fn list_mic_devices() -> Result<Vec<String>> {
    std::thread::spawn(|| -> Result<Vec<String>> {
        let _ = wasapi::initialize_mta();
        let devices = DeviceEnumerator::new()?.get_device_collection(&Direction::Capture)?;
        let mut names = Vec::new();
        for dev in &devices {
            if let Ok(name) = dev?.get_friendlyname() {
                names.push(name);
            }
        }
        Ok(names)
    })
    .join()
    .map_err(|_| anyhow::anyhow!("mic enumeration panicked"))?
}

fn open_client(source: &AudioSource) -> Result<AudioClient> {
    match source {
        AudioSource::Process { pid } => AudioClient::new_application_loopback_client(*pid, true)
            .with_context(|| format!("process loopback for pid {pid}")),
        // A render device opened for capture is a loopback stream.
        AudioSource::Desktop => Ok(DeviceEnumerator::new()?
            .get_default_device(&Direction::Render)
            .context("no default output device")?
            .get_iaudioclient()?),
        AudioSource::Mic { device } => {
            let enumerator = DeviceEnumerator::new()?;
            let dev = match device {
                Some(name) => enumerator
                    .get_device_collection(&Direction::Capture)?
                    .get_device_with_name(name)
                    .with_context(|| format!("mic \"{name}\" not found"))?,
                None => enumerator.get_default_device(&Direction::Capture)?,
            };
            Ok(dev.get_iaudioclient()?)
        }
    }
}

fn run(source: AudioSource, server: &PipeServer, stop: &AtomicBool, origin: Instant) -> Result<()> {
    server.wait_for_client()?;
    if stop.load(Ordering::Relaxed) {
        return Ok(());
    }
    let _ = wasapi::initialize_mta();

    // If the source can't be opened (no mic plugged in, etc.) keep feeding
    // silence so the track stays valid for the whole recording.
    let stream = match Stream::open(&source) {
        Ok(s) => Some(s),
        Err(e) => {
            tracing::warn!("{source:?} unavailable, recording silence: {e:#}");
            None
        }
    };

    let mut written: u64 = 0;
    let mut packet = Vec::new();
    let silence = vec![0u8; BYTES_PER_FRAME * MAX_AHEAD_FRAMES as usize];
    let mut stats = Stats::default();

    while !stop.load(Ordering::Relaxed) {
        stats.maybe_log(&source);
        // Fill gaps so the timeline never stalls. Done before reading, so
        // the first packets land at the time they were captured, not at the
        // origin.
        let expected = clock_frames(origin);
        while written + MAX_BEHIND_FRAMES < expected {
            let n = (expected - written).min(MAX_AHEAD_FRAMES) as usize;
            server.write_all(&silence[..n * BYTES_PER_FRAME])?;
            written += n as u64;
            stats.padded += n as u64;
        }

        match &stream {
            Some(s) => {
                // Times out every 10 ms when there is nothing to read; that's our tick.
                let _ = s.event.wait_for_event(10);
                while let Some(frames) = s.capture.get_next_packet_size()? {
                    if frames == 0 {
                        break;
                    }
                    packet.resize(frames as usize * BYTES_PER_FRAME, 0);
                    let (got, info) = s.capture.read_from_device(&mut packet)?;
                    if got == 0 {
                        break;
                    }
                    let bytes = &mut packet[..got as usize * BYTES_PER_FRAME];
                    if info.flags.silent {
                        bytes.fill(0);
                    }
                    stats.observe(bytes, info.flags.silent);
                    let ahead_limit = clock_frames(origin) + MAX_AHEAD_FRAMES;
                    if written >= ahead_limit {
                        stats.dropped += got as u64;
                        continue; // late burst after we already padded: drop it
                    }
                    let keep = (ahead_limit - written).min(got as u64) as usize;
                    stats.dropped += got as u64 - keep as u64;
                    server.write_all(&bytes[..keep * BYTES_PER_FRAME])?;
                    written += keep as u64;
                }
            }
            None => std::thread::sleep(Duration::from_millis(10)),
        }
    }
    Ok(())
}

/// Periodic debug counters, to diagnose silent or glitchy tracks.
struct Stats {
    since: Instant,
    captured: u64,
    silent_packets: u64,
    padded: u64,
    dropped: u64,
    peak: f32,
}

impl Default for Stats {
    fn default() -> Self {
        Self { since: Instant::now(), captured: 0, silent_packets: 0, padded: 0, dropped: 0, peak: 0.0 }
    }
}

impl Stats {
    fn observe(&mut self, bytes: &[u8], silent: bool) {
        self.captured += (bytes.len() / BYTES_PER_FRAME) as u64;
        self.silent_packets += silent as u64;
        for s in bytes.as_chunks::<4>().0 {
            self.peak = self.peak.max(f32::from_le_bytes(*s).abs());
        }
    }

    fn maybe_log(&mut self, source: &AudioSource) {
        if self.since.elapsed() >= Duration::from_secs(5) {
            tracing::debug!(
                "{source:?}: captured {} padded {} dropped {} frames, {} silent packets, peak {:.3}",
                self.captured,
                self.padded,
                self.dropped,
                self.silent_packets,
                self.peak
            );
            *self = Self::default();
        }
    }
}

struct Stream {
    client: AudioClient,
    capture: AudioCaptureClient,
    event: Handle,
}

impl Stream {
    fn open(source: &AudioSource) -> Result<Self> {
        let format = WaveFormat::new(32, 32, &SampleType::Float, AUDIO_RATE as usize, AUDIO_CHANNELS as usize, None);
        let mut client = open_client(source)?;
        let mode = StreamMode::EventsShared { autoconvert: true, buffer_duration_hns: 0 };
        client.initialize_client(&format, &Direction::Capture, &mode)?;
        let event = client.set_get_eventhandle()?;
        let capture = client.get_audiocaptureclient()?;
        client.start_stream()?;
        Ok(Self { client, capture, event })
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        let _ = self.client.stop_stream();
    }
}

fn clock_frames(origin: Instant) -> u64 {
    let e: Duration = origin.elapsed();
    e.as_micros() as u64 * AUDIO_RATE as u64 / 1_000_000
}

/// Unique pipe path for one input of one recording.
pub fn pipe_name(tag: &str) -> String {
    use std::sync::atomic::AtomicU64;
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!(r"\\.\pipe\playerstv-{}-{n}-{tag}", std::process::id())
}
