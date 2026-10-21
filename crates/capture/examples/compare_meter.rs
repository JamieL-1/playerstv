//! Over N seconds, compares a process's session peak meter (what Windows says
//! it plays) with what process loopback captures from it.
//! usage: compare_meter <pid> [seconds]
use std::time::{Duration, Instant};
use wasapi::*;
use windows::core::Interface;
use windows::Win32::Media::Audio::Endpoints::IAudioMeterInformation;
use windows::Win32::Media::Audio::{IAudioSessionControl2, IAudioSessionManager2};
use windows::Win32::System::Com::CLSCTX_ALL;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _ = initialize_mta();
    let mut args = std::env::args().skip(1);
    let pid: u32 = args.next().expect("pid").parse()?;
    let secs: u64 = args.next().map(|s| s.parse()).transpose()?.unwrap_or(15);

    let dev = DeviceEnumerator::new()?.get_default_device(&Direction::Render)?;
    let meters: Vec<IAudioMeterInformation> = unsafe {
        let mgr: IAudioSessionManager2 = dev.as_immdevice().Activate(CLSCTX_ALL, None)?;
        let e = mgr.GetSessionEnumerator()?;
        (0..e.GetCount()?)
            .filter_map(|i| e.GetSession(i).ok())
            .filter(|s| s.cast::<IAudioSessionControl2>().ok().and_then(|c| c.GetProcessId().ok()) == Some(pid))
            .filter_map(|s| s.cast().ok())
            .collect()
    };

    let fmt = WaveFormat::new(32, 32, &SampleType::Float, 48000, 2, None);
    let mut client = AudioClient::new_application_loopback_client(pid, true)?;
    client.initialize_client(
        &fmt,
        &Direction::Capture,
        &StreamMode::EventsShared { autoconvert: true, buffer_duration_hns: 0 },
    )?;
    let ev = client.set_get_eventhandle()?;
    let cap = client.get_audiocaptureclient()?;
    client.start_stream()?;

    let (mut meter_max, mut cap_max) = (0f32, 0f32);
    let mut buf = vec![0u8; 1 << 20];
    let t = Instant::now();
    let mut next_report = Duration::from_secs(1);
    let (mut sec_meter, mut sec_cap) = (0f32, 0f32);
    while t.elapsed() < Duration::from_secs(secs) {
        let _ = ev.wait_for_event(20);
        while let Some(f) = cap.get_next_packet_size()? {
            if f == 0 {
                break;
            }
            let (got, _) = cap.read_from_device(&mut buf)?;
            for s in buf[..got as usize * 8].as_chunks::<4>().0 {
                sec_cap = sec_cap.max(f32::from_le_bytes(*s).abs());
            }
        }
        for m in &meters {
            sec_meter = sec_meter.max(unsafe { m.GetPeakValue().unwrap_or(0.0) });
        }
        if t.elapsed() >= next_report {
            println!("{:>3}s  meter {sec_meter:.3}  captured {sec_cap:.3}", next_report.as_secs());
            meter_max = meter_max.max(sec_meter);
            cap_max = cap_max.max(sec_cap);
            sec_meter = 0.0;
            sec_cap = 0.0;
            next_report += Duration::from_secs(1);
        }
    }
    println!("max: meter {meter_max:.3}, captured {cap_max:.3} ({} session(s) on default device)", meters.len());
    Ok(())
}
