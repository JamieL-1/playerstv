use ptv_core::profile::{GameProfile, ProfileMatcher};
use windows::core::{BOOL, PWSTR};
use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, RECT};
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetClientRect, GetWindowTextW, GetWindowThreadProcessId, IsIconic, IsWindow,
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
    let mut windows: Vec<(HWND, u32, String)> = Vec::new();
    unsafe {
        let _ = EnumWindows(
            Some(collect_window),
            LPARAM(&mut windows as *mut Vec<(HWND, u32, String)> as isize),
        );
    }
    for m in matchers {
        for (hwnd, pid, title) in &windows {
            let Some(exe) = exe_name(*pid) else { continue };
            if m.matches(&exe, title) {
                return Some(GameWindow {
                    hwnd: hwnd.0 as u64,
                    pid: *pid,
                    title: title.clone(),
                    profile: m.profile.clone(),
                });
            }
        }
    }
    None
}

unsafe extern "system" fn collect_window(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let out = &mut *(lparam.0 as *mut Vec<(HWND, u32, String)>);
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
    out.push((hwnd, pid, String::from_utf16_lossy(&buf[..len as usize])));
    true.into()
}

/// File name of the process's executable, e.g. "RuneLite.exe".
pub fn exe_name(pid: u32) -> Option<String> {
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
    unsafe { IsWindow(Some(HWND(hwnd as *mut _))).as_bool() }
}

/// Client-area size, or `(0, 0)` while minimised.
pub fn client_size(hwnd: u64) -> (u32, u32) {
    let hwnd = HWND(hwnd as *mut _);
    unsafe {
        if IsIconic(hwnd).as_bool() {
            return (0, 0);
        }
        let mut rect = RECT::default();
        if GetClientRect(hwnd, &mut rect).is_err() {
            return (0, 0);
        }
        (
            (rect.right - rect.left).max(0) as u32,
            (rect.bottom - rect.top).max(0) as u32,
        )
    }
}
