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
