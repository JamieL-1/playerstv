//! Prints, for every output device, each audio session's process id, peak
//! level and per-app mixer volume. Handy when a captured game track is
//! unexpectedly quiet: process loopback records *after* the mixer volume.
use windows::core::Interface;
use windows::Win32::Media::Audio::Endpoints::IAudioMeterInformation;
use windows::Win32::Media::Audio::{IAudioSessionControl2, IAudioSessionManager2, ISimpleAudioVolume};
use windows::Win32::System::Com::CLSCTX_ALL;
use wasapi::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _ = initialize_mta();
    let enumerator = DeviceEnumerator::new()?;
    let default_id = enumerator.get_default_device(&Direction::Render)?.get_id()?;
    for dev in &enumerator.get_device_collection(&Direction::Render)? {
        let dev = dev?;
        let mark = if dev.get_id()? == default_id { " (default)" } else { "" };
        println!("{}{mark}", dev.get_friendlyname()?);
        unsafe {
            let mgr: IAudioSessionManager2 = dev.as_immdevice().Activate(CLSCTX_ALL, None)?;
            let sessions = mgr.GetSessionEnumerator()?;
            for i in 0..sessions.GetCount()? {
                let ctl = sessions.GetSession(i)?;
                let ctl2: IAudioSessionControl2 = ctl.cast()?;
                let vol: ISimpleAudioVolume = ctl.cast()?;
                let meter: IAudioMeterInformation = ctl.cast()?;
                println!(
                    "  pid {:>6}  peak {:.3}  mixer volume {:>3.0}%  muted {}",
                    ctl2.GetProcessId().unwrap_or(0),
                    meter.GetPeakValue().unwrap_or(-1.0),
                    vol.GetMasterVolume()? * 100.0,
                    vol.GetMute()?.as_bool()
                );
            }
        }
    }
    Ok(())
}
