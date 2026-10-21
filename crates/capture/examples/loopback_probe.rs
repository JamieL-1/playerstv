//! Captures ~2s from a process (and, for comparison, the whole output device)
//! with a few formats and prints levels. Diagnoses quiet "Game" tracks.
//! usage: loopback_probe <pid>
use std::time::{Duration, Instant};
use wasapi::*;

fn probe(label: &str, mut client: AudioClient, fmt: &WaveFormat) -> Result<(), Box<dyn std::error::Error>> {
    let mode = StreamMode::EventsShared { autoconvert: true, buffer_duration_hns: 0 };
    client.initialize_client(fmt, &Direction::Capture, &mode)?;
    let ev = client.set_get_eventhandle()?;
    let cap = client.get_audiocaptureclient()?;
    client.start_stream()?;
    let (mut peak, mut sum, mut n, mut silent) = (0f64, 0f64, 0u64, 0u64);
    let bits = fmt.get_bitspersample();
    let t = Instant::now();
    let mut buf = vec![0u8; 1 << 20];
    while t.elapsed() < Duration::from_secs(2) {
        let _ = ev.wait_for_event(100);
        while let Some(f) = cap.get_next_packet_size()? {
            if f == 0 {
                break;
            }
            let (got, info) = cap.read_from_device(&mut buf)?;
            if info.flags.silent {
                silent += 1;
            }
            let bytes = &buf[..got as usize * fmt.get_blockalign() as usize];
            if bits == 32 {
                for s in bytes.as_chunks::<4>().0 {
                    let v = f32::from_le_bytes(*s) as f64;
                    peak = peak.max(v.abs());
                    sum += v * v;
                    n += 1;
                }
            } else {
                for s in bytes.as_chunks::<2>().0 {
                    let v = i16::from_le_bytes(*s) as f64 / 32768.0;
                    peak = peak.max(v.abs());
                    sum += v * v;
                    n += 1;
                }
            }
        }
    }
    client.stop_stream()?;
    let rms = (sum / n.max(1) as f64).sqrt();
    println!("{label:<34} samples {n:>7}  peak {peak:.3}  rms {rms:.4}  silent pkts {silent}");
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _ = initialize_mta();
    let pid: u32 = std::env::args().nth(1).expect("pid").parse()?;
    let f32fmt = WaveFormat::new(32, 32, &SampleType::Float, 48000, 2, None);
    let i16fmt = WaveFormat::new(16, 16, &SampleType::Int, 48000, 2, None);
    let i16_44 = WaveFormat::new(16, 16, &SampleType::Int, 44100, 2, None);
    probe("process tree, f32 48k", AudioClient::new_application_loopback_client(pid, true)?, &f32fmt)?;
    probe("process tree, i16 48k", AudioClient::new_application_loopback_client(pid, true)?, &i16fmt)?;
    probe("process tree, i16 44.1k", AudioClient::new_application_loopback_client(pid, true)?, &i16_44)?;
    probe("EXCLUDE tree, f32 48k", AudioClient::new_application_loopback_client(pid, false)?, &f32fmt)?;
    let dev = DeviceEnumerator::new()?.get_default_device(&Direction::Render)?;
    probe("whole device loopback, f32 48k", dev.get_iaudioclient()?, &f32fmt)?;
    Ok(())
}
