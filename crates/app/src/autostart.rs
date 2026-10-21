//! "Start with Windows" via the per-user Run registry key.

use anyhow::Result;
use winreg::enums::HKEY_CURRENT_USER;
use winreg::RegKey;

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const VALUE: &str = "playerstv";

pub fn apply(enabled: bool) -> Result<()> {
    let (run, _) = RegKey::predef(HKEY_CURRENT_USER).create_subkey(RUN_KEY)?;
    if enabled {
        let exe = std::env::current_exe()?;
        run.set_value(VALUE, &format!("\"{}\"", exe.display()))?;
    } else if run.get_raw_value(VALUE).is_ok() {
        run.delete_value(VALUE)?;
    }
    Ok(())
}
