use std::collections::HashMap;

use ptv_core::profile::{GameProfile, ProfileMatcher};
use windows::core::{BOOL, PWSTR};
use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, RECT};
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetClientRect, GetForegroundWindow, GetWindowTextW, GetWindowThreadProcessId, IsIconic, IsWindow,
    IsWindowVisible,
};

/// A running game window that matched one of the profiles.
#[derive(Debug, Clone)]
pub struct GameWindow {
    pub hwnd: u64,
    pub pid: u32,
    pub title: String,
    pub profile: GameProfile,
}

/// Scans top-level windows and returns the first one matching a profile.
/// Profiles earlier in the list win.
pub fn find_game(matchers: &[ProfileMatcher]) -> Option<GameWindow> {
    let windows = visible_windows();
    // Title regexes are cheap; looking up a process's exe is a system call.
    // So check titles first and look each process's exe up at most once,
    // only for windows whose title some profile matches. That's usually
    // none, but a custom game's `.*` matches every window, and one process
    // (Explorer, a browser) often owns many of them.
    let mut exes: HashMap<u32, Option<String>> = HashMap::new();
    for m in matchers {
        for w in &windows {
            if !m.title_matches(&w.title) {
                continue;
            }
            let exe = exes.entry(w.pid).or_insert_with(|| exe_name(w.pid));
            if exe.as_deref().is_some_and(|e| m.matches(e, &w.title)) {
                return Some(GameWindow {
                    hwnd: w.hwnd.0 as u64,
                    pid: w.pid,
                    title: w.title.clone(),
                    profile: m.profile.clone(),
                });
            }
        }
    }
    None
}

/// A visible top-level window with exactly this title, owned by another
/// process running `exe`. Matching on the executable too means e.g. an
/// Explorer window that happens to have the same title is never picked.
pub fn find_window(title: &str, exe: &str) -> Option<u64> {
    let me = std::process::id();
    visible_windows()
        .into_iter()
        .filter(|w| w.title == title && w.pid != me)
        .find(|w| exe_name(w.pid).is_some_and(|e| e.eq_ignore_ascii_case(exe)))
        .map(|w| w.hwnd.0 as u64)
}

/// A visible, titled top-level window.
struct TopWindow {
    hwnd: HWND,
    pid: u32,
    title: String,
}

fn visible_windows() -> Vec<TopWindow> {
    let mut windows: Vec<TopWindow> = Vec::new();
    // SAFETY: EnumWindows calls `collect_window` synchronously, on this
    // thread, before returning, so the pointer to `windows` it passes along
    // stays valid and unaliased for every call.
    unsafe {
        let _ = EnumWindows(Some(collect_window), LPARAM(&mut windows as *mut Vec<TopWindow> as isize));
    }
    windows
}

/// # Safety
///
/// `lparam` must point to a `Vec<TopWindow>` that nothing else accesses
/// during the call (see [`visible_windows`]). A panic here can't unwind into
/// Windows: since Rust 1.81 it aborts at the `extern "system"` boundary.
unsafe extern "system" fn collect_window(hwnd: HWND, lparam: LPARAM) -> BOOL {
    // SAFETY: guaranteed by the caller, as documented above.
    let out = unsafe { &mut *(lparam.0 as *mut Vec<TopWindow>) };
    // SAFETY: `hwnd` comes from EnumWindows; a window that has closed since
    // just makes these calls fail. `buf` and `pid` outlive the calls.
    unsafe {
        if !IsWindowVisible(hwnd).as_bool() {
            return true.into();
        }
        let mut buf = [0u16; 512];
        let len = GetWindowTextW(hwnd, &mut buf);
        if len <= 0 {
            return true.into();
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        out.push(TopWindow { hwnd, pid, title: String::from_utf16_lossy(&buf[..len as usize]) });
    }
    true.into()
}

/// File name of the process's executable, e.g. "RuneLite.exe".
pub fn exe_name(pid: u32) -> Option<String> {
    // SAFETY: the process handle is checked by `.ok()?` and closed exactly
    // once; `buf` outlives the call and `len` tells the API its capacity in
    // UTF-16 units, which the API then sets to the length it wrote.
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(handle, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len);
        let _ = CloseHandle(handle);
        ok.ok()?;
        let path = String::from_utf16_lossy(&buf[..len as usize]);
        path.rsplit(['\\', '/']).next().map(str::to_owned)
    }
}

pub fn window_alive(hwnd: u64) -> bool {
    // SAFETY: IsWindow accepts any value, including stale handles.
    unsafe { IsWindow(Some(HWND(hwnd as *mut _))).as_bool() }
}

/// Whether the window is the one the player is using.
pub fn is_foreground(hwnd: u64) -> bool {
    // SAFETY: GetForegroundWindow takes no arguments and only reads.
    unsafe { GetForegroundWindow().0 as u64 == hwnd }
}

/// Client-area size, or `(0, 0)` while minimised.
pub fn client_size(hwnd: u64) -> (u32, u32) {
    let hwnd = HWND(hwnd as *mut _);
    // SAFETY: both calls fail cleanly for a window that has closed, and
    // `rect` outlives the call that fills it in.
    unsafe {
        if IsIconic(hwnd).as_bool() {
            return (0, 0);
        }
        let mut rect = RECT::default();
        if GetClientRect(hwnd, &mut rect).is_err() {
            return (0, 0);
        }
        ((rect.right - rect.left).max(0) as u32, (rect.bottom - rect.top).max(0) as u32)
    }
}
