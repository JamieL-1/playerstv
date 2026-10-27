//! A borderless top-level window and its message pump.

use anyhow::{Context, Result};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};
use windows::Win32::UI::Input::KeyboardAndMouse::VK_ESCAPE;
use windows::Win32::UI::WindowsAndMessaging::{
    AdjustWindowRectEx, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetSystemMetrics,
    LoadCursorW, PeekMessageW, PostQuitMessage, RegisterClassW, TranslateMessage, IDC_ARROW, MSG, PM_REMOVE,
    SM_CXSCREEN, SM_CYSCREEN, WM_DESTROY, WM_KEYDOWN, WM_QUIT, WNDCLASSW, WS_CAPTION, WS_EX_APPWINDOW, WS_POPUP,
    WS_SYSMENU, WS_VISIBLE,
};

/// The title recorders and playerstv's game profile match on.
pub const TITLE: PCWSTR = w!("ptv-bench-game");

pub struct Window {
    pub hwnd: HWND,
}

impl Window {
    /// Without a size: borderless and covering the primary monitor, like a
    /// game in borderless fullscreen. With one: a normal window whose
    /// client area is roughly that size.
    pub fn open(size: Option<(u32, u32)>) -> Result<(Self, u32, u32)> {
        // Physical pixels, so a scaled desktop doesn't stretch our frames.
        // SAFETY: plain FFI call with a predefined context constant. It fails
        // harmlessly if awareness was already set.
        let _ = unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };

        // SAFETY: plain FFI calls; the class name and title are 'static wide
        // strings from `w!`, `wndproc` matches WNDPROC, and `outer` is a
        // valid local for AdjustWindowRectEx.
        unsafe {
            let instance = GetModuleHandleW(None).context("GetModuleHandleW")?;
            let class = WNDCLASSW {
                lpfnWndProc: Some(wndproc),
                hInstance: instance.into(),
                hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
                lpszClassName: TITLE,
                ..Default::default()
            };
            if RegisterClassW(&class) == 0 {
                anyhow::bail!("RegisterClassW: {}", windows::core::Error::from_thread());
            }
            let (style, w, h) = match size {
                None => (WS_POPUP, GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN)),
                Some((w, h)) => (WS_CAPTION | WS_SYSMENU, w as i32, h as i32),
            };
            // Grow the window so its client area is exactly w×h.
            let mut outer = RECT { left: 0, top: 0, right: w, bottom: h };
            AdjustWindowRectEx(&mut outer, style, false, WS_EX_APPWINDOW).context("AdjustWindowRectEx")?;
            let hwnd = CreateWindowExW(
                WS_EX_APPWINDOW,
                TITLE,
                TITLE,
                style | WS_VISIBLE,
                0,
                0,
                outer.right - outer.left,
                outer.bottom - outer.top,
                None,
                None,
                Some(instance.into()),
                None,
            )
            .context("CreateWindowExW")?;
            Ok((Self { hwnd }, w as u32, h as u32))
        }
    }

    /// Handles queued messages. `false` once the window has been closed.
    pub fn pump(&self) -> bool {
        let mut msg = MSG::default();
        // SAFETY: `msg` is a valid out-pointer for each call; messages are
        // translated and dispatched on the thread that created the window.
        unsafe {
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                if msg.message == WM_QUIT {
                    return false;
                }
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        true
    }
}

impl Drop for Window {
    fn drop(&mut self) {
        // SAFETY: we created `hwnd` on this thread; it may already be gone
        // if the user closed it, which DestroyWindow reports as an error.
        let _ = unsafe { DestroyWindow(self.hwnd) };
    }
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_KEYDOWN if wparam.0 == VK_ESCAPE.0 as usize => {
            // SAFETY: called by Windows with our own window handle.
            let _ = unsafe { DestroyWindow(hwnd) };
            LRESULT(0)
        }
        WM_DESTROY => {
            // SAFETY: plain FFI call on the window's thread.
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        // SAFETY: forwarding the arguments Windows gave us, unchanged.
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}
