//! Finding and closing the test game's window from the harness.

use windows::core::w;
use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, PostMessageW, WM_CLOSE};

/// Asks the test game to close, as clicking its close button would, so it
/// writes its frame log on the way out. `false` if it isn't open.
pub fn close_game() -> bool {
    // SAFETY: plain FFI calls with a 'static class name; posting WM_CLOSE to
    // another process's window is allowed and doesn't wait for it.
    unsafe {
        match FindWindowW(w!("ptv-bench-game"), None) {
            Ok(hwnd) => PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0)).is_ok(),
            Err(_) => false,
        }
    }
}

/// Waits up to `timeout` for the test game's window, brings it to the
/// foreground and checks it got there. A game in the background is
/// captured differently from one being played: Windows Graphics Capture
/// delivered about 73% of its frames in the background and 90% in front.
pub fn focus_game(timeout: std::time::Duration) -> anyhow::Result<()> {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP, VK_MENU,
    };
    use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, SetForegroundWindow};

    let deadline = std::time::Instant::now() + timeout;
    loop {
        // SAFETY: plain FFI calls with a 'static class name. The INPUTs are
        // fully initialised keyboard events and the size is one INPUT's.
        unsafe {
            if let Ok(hwnd) = FindWindowW(w!("ptv-bench-game"), None) {
                if GetForegroundWindow() == hwnd {
                    return Ok(());
                }
                // Windows only lets the process that sent the last input
                // change the foreground window, so tap Alt first, as a
                // player alt-tabbing back would.
                let key = |flags| INPUT {
                    r#type: INPUT_KEYBOARD,
                    Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: VK_MENU, dwFlags: flags, ..Default::default() } },
                };
                SendInput(&[key(KEYBD_EVENT_FLAGS(0)), key(KEYEVENTF_KEYUP)], size_of::<INPUT>() as i32);
                let _ = SetForegroundWindow(hwnd);
            }
        }
        if std::time::Instant::now() >= deadline {
            anyhow::bail!("couldn't bring the test game to the foreground");
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

/// Whether the test game's window is the foreground window.
pub fn game_has_focus() -> bool {
    use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;
    // SAFETY: plain FFI calls with a 'static class name.
    unsafe { FindWindowW(w!("ptv-bench-game"), None).is_ok_and(|hwnd| GetForegroundWindow() == hwnd) }
}

/// The primary monitor's size in physical pixels: what the test game
/// covers when it runs borderless.
pub fn primary_monitor_size() -> (u32, u32) {
    use windows::Win32::UI::HiDpi::{SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};
    use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN};
    // SAFETY: plain FFI calls. Setting awareness fails harmlessly if it's
    // already set; without it a scaled desktop reports scaled sizes.
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        (GetSystemMetrics(SM_CXSCREEN) as u32, GetSystemMetrics(SM_CYSCREEN) as u32)
    }
}
