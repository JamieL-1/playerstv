//! Pressing a recorder's save hotkey the way a player would, so save
//! latency covers the same path for every recorder.

use anyhow::{bail, Result};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP, VIRTUAL_KEY,
};

/// Virtual-key codes, modifiers first, for e.g. `Alt+F10` or `Win+Alt+G`.
pub fn parse(hotkey: &str) -> Result<Vec<u16>> {
    let mut keys = Vec::new();
    for part in hotkey.split('+').map(str::trim) {
        let upper = part.to_ascii_uppercase();
        let vk = match upper.as_str() {
            "CTRL" | "CONTROL" => 0x11,
            "ALT" => 0x12,
            "SHIFT" => 0x10,
            "WIN" | "META" | "SUPER" => 0x5B,
            f if f.len() > 1 && f.starts_with('F') => match f[1..].parse::<u16>() {
                Ok(n @ 1..=24) => 0x70 + n - 1,
                _ => bail!("unknown key {part:?} in {hotkey:?}"),
            },
            k if k.len() == 1 && k.as_bytes()[0].is_ascii_alphanumeric() => k.as_bytes()[0] as u16,
            _ => bail!("unknown key {part:?} in {hotkey:?}"),
        };
        keys.push(vk);
    }
    if keys.is_empty() {
        bail!("empty hotkey");
    }
    Ok(keys)
}

/// Presses the keys in order and releases them in reverse.
pub fn press(hotkey: &str) -> Result<()> {
    let keys = parse(hotkey)?;
    let event = |vk: u16, flags: KEYBD_EVENT_FLAGS| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: VIRTUAL_KEY(vk), dwFlags: flags, ..Default::default() } },
    };
    let inputs: Vec<INPUT> = keys
        .iter()
        .map(|&vk| event(vk, KEYBD_EVENT_FLAGS(0)))
        .chain(keys.iter().rev().map(|&vk| event(vk, KEYEVENTF_KEYUP)))
        .collect();
    // SAFETY: `inputs` is a slice of fully initialised keyboard INPUTs and
    // the size passed is the size of one INPUT, as SendInput requires.
    let sent = unsafe { SendInput(&inputs, size_of::<INPUT>() as i32) };
    if sent as usize != inputs.len() {
        bail!("SendInput sent {sent} of {} key events for {hotkey}", inputs.len());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_hotkeys() {
        assert_eq!(parse("Alt+F10").unwrap(), [0x12, 0x79]);
        assert_eq!(parse("Win + Alt + G").unwrap(), [0x5B, 0x12, b'G' as u16]);
        assert_eq!(parse("ctrl+shift+f24").unwrap(), [0x11, 0x10, 0x87]);
        assert_eq!(parse("F8").unwrap(), [0x77]);
        assert!(parse("Alt+F25").is_err());
        assert!(parse("Alt+Enter").is_err());
    }
}
