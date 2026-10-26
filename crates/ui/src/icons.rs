//! Game icons for the rail: the icon of each game's own executable, cached
//! as a PNG under `%LOCALAPPDATA%\playerstv\icons\<game id>.png`.
//!
//! An exe is found from a running process with one of the game's exe names,
//! or (built-ins) where the game is normally installed, or (games added with
//! the rail's "+") from the exe the user picked. Games without one keep
//! their initials until they're next seen running.

use std::collections::HashMap;
use std::ffi::c_void;
use std::path::{Path, PathBuf};

use base64::Engine;
use ptv_common::paths;
use ptv_core::config::Config;
use ptv_core::profile::{self, GameProfile};
use tauri::AppHandle;
use tauri_plugin_dialog::DialogExt;
use windows::core::{HSTRING, PWSTR};
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, DeleteDC, DeleteObject, GetDIBits, GetObjectW, BITMAP, BITMAPINFO, BITMAPINFOHEADER, BI_RGB,
    DIB_RGB_COLORS, HBITMAP, HGDIOBJ,
};
use windows::Win32::Storage::FileSystem::{GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::Shell::SHDefExtractIconW;
use windows::Win32::UI::WindowsAndMessaging::{DestroyIcon, GetIconInfo, HICON, ICONINFO};

/// Big enough for the 48 px rail bubble on a 200% display.
const ICON_SIZE: u32 = 128;

/// Generic hosts (RuneLite can run as java.exe): their icon isn't the game's.
const HOSTS: [&str; 2] = ["java.exe", "javaw.exe"];

/// Game id → `data:image/png` URL, for every game whose icon is known.
#[tauri::command]
pub async fn game_icons() -> Result<HashMap<String, String>, String> {
    let cfg = Config::load_or_create(&paths::config_file()).map_err(|e| format!("{e:#}"))?;
    tauri::async_runtime::spawn_blocking(move || resolve_all(&cfg.games)).await.map_err(|e| e.to_string())
}

fn resolve_all(games: &[GameProfile]) -> HashMap<String, String> {
    let dir = paths::icons_dir();
    let _ = std::fs::create_dir_all(&dir);
    let missing: Vec<&GameProfile> = games.iter().filter(|g| !dir.join(format!("{}.png", g.id)).exists()).collect();
    if !missing.is_empty() {
        let running = running_exes(&missing);
        for g in missing {
            if let Some(exe) = find_exe(g, &running) {
                let _ = cache_icon(&exe, &dir.join(format!("{}.png", g.id)));
            }
        }
    }
    let engine = base64::engine::general_purpose::STANDARD;
    games
        .iter()
        .filter_map(|g| {
            let png = std::fs::read(dir.join(format!("{}.png", g.id))).ok()?;
            Some((g.id.clone(), format!("data:image/png;base64,{}", engine.encode(png))))
        })
        .collect()
}

/// The new game's id, or `None` if the user cancelled the picker.
#[tauri::command]
pub async fn add_game(app: AppHandle) -> Result<Option<String>, String> {
    let picked = app
        .dialog()
        .file()
        .set_title("Choose the game's .exe")
        .add_filter("Programs", &["exe"])
        .blocking_pick_file()
        .and_then(|p| p.into_path().ok());
    let Some(exe) = picked else { return Ok(None) };
    let exe_name = exe.file_name().map(|n| n.to_string_lossy().into_owned()).ok_or("not a file")?;

    let path = paths::config_file();
    let mut cfg = Config::load_or_create(&path).map_err(|e| format!("{e:#}"))?;
    let name = display_name(&exe);
    let used: Vec<String> = cfg.games.iter().map(|g| g.id.clone()).collect();
    let id = profile::unique_custom_id(&name, &used);
    cfg.games.push(GameProfile {
        id: id.clone(),
        name,
        exe_names: vec![exe_name],
        window_title_regex: ".*".into(),
        enabled: true,
        overrides: Default::default(),
    });
    cfg.sanitize();
    // Before saving, so the rail's refresh on `config-changed` finds it.
    let _ = std::fs::create_dir_all(paths::icons_dir());
    if let Err(e) = cache_icon(&exe, &paths::icons_dir().join(format!("{id}.png"))) {
        tracing::warn!("no icon for {}: {e}", exe.display());
    }
    cfg.save(&path).map_err(|e| format!("{e:#}"))?;
    Ok(Some(id))
}

fn find_exe(game: &GameProfile, running: &[PathBuf]) -> Option<PathBuf> {
    let wanted = |p: &PathBuf| {
        let name = p.file_name().map(|n| n.to_string_lossy()).unwrap_or_default();
        !HOSTS.iter().any(|h| h.eq_ignore_ascii_case(&name))
            && game.exe_names.iter().any(|e| e.eq_ignore_ascii_case(&name))
    };
    running.iter().find(|p| wanted(p)).cloned().or_else(|| install_paths(&game.id).into_iter().find(|p| p.is_file()))
}

/// Where built-in games are normally installed.
fn install_paths(id: &str) -> Vec<PathBuf> {
    let env = |var: &str| std::env::var_os(var).map(PathBuf::from);
    match id {
        profile::RUNELITE => [env("LOCALAPPDATA"), env("ProgramFiles")]
            .into_iter()
            .flatten()
            .map(|d| d.join("RuneLite").join("RuneLite.exe"))
            .collect(),
        profile::LEAGUE => {
            let installed = reg_string(
                winreg::enums::HKEY_CURRENT_USER,
                r"Software\Microsoft\Windows\CurrentVersion\Uninstall\Riot Game league_of_legends.live",
                "InstallLocation",
            );
            installed
                .map(PathBuf::from)
                .into_iter()
                .chain([PathBuf::from(r"C:\Riot Games\League of Legends")])
                .map(|d| d.join("Game").join("League of Legends.exe"))
                .collect()
        }
        profile::CS2 => steam_libraries()
            .into_iter()
            .map(|lib| lib.join(r"steamapps\common\Counter-Strike Global Offensive\game\bin\win64\cs2.exe"))
            .collect(),
        _ => Vec::new(),
    }
}

fn reg_string(root: winreg::HKEY, key: &str, value: &str) -> Option<String> {
    winreg::RegKey::predef(root).open_subkey(key).ok()?.get_value(value).ok()
}

/// Steam's own folder plus every library listed in `libraryfolders.vdf`.
fn steam_libraries() -> Vec<PathBuf> {
    let Some(steam) = reg_string(winreg::enums::HKEY_CURRENT_USER, r"Software\Valve\Steam", "SteamPath") else {
        return Vec::new();
    };
    let steam = PathBuf::from(steam);
    let vdf = std::fs::read_to_string(steam.join(r"steamapps\libraryfolders.vdf")).unwrap_or_default();
    let mut libs = vec![steam];
    libs.extend(vdf_paths(&vdf).map(PathBuf::from));
    libs
}

/// The `"path"  "C:\\…"` values of a `libraryfolders.vdf`.
fn vdf_paths(vdf: &str) -> impl Iterator<Item = String> + '_ {
    vdf.lines().filter_map(|line| {
        let mut quoted = line.split('"').skip(1).step_by(2);
        (quoted.next()? == "path").then(|| quoted.next().map(|p| p.replace(r"\\", r"\")))?
    })
}

/// Full paths of the running processes (those we may query) whose exe one
/// of `games` uses. The snapshot already names every exe, so only those
/// processes are opened for their full path, not all few hundred.
fn running_exes(games: &[&GameProfile]) -> Vec<PathBuf> {
    let wanted = |exe: &str| games.iter().any(|g| g.exe_names.iter().any(|e| e.eq_ignore_ascii_case(exe)));
    let mut out = Vec::new();
    // SAFETY: `entry` is a properly sized PROCESSENTRY32W (`dwSize` set as the
    // API requires) that outlives every call writing to it, and the snapshot
    // handle is closed exactly once, after its last use.
    unsafe {
        let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else { return out };
        let mut entry = PROCESSENTRY32W { dwSize: size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        let mut more = Process32FirstW(snap, &mut entry).is_ok();
        while more {
            let name = &entry.szExeFile;
            let name = String::from_utf16_lossy(&name[..name.iter().position(|&c| c == 0).unwrap_or(name.len())]);
            if wanted(&name) {
                out.extend(process_path(entry.th32ProcessID));
            }
            more = Process32NextW(snap, &mut entry).is_ok();
        }
        let _ = CloseHandle(snap);
    }
    out
}

fn process_path(pid: u32) -> Option<PathBuf> {
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
        Some(PathBuf::from(String::from_utf16_lossy(&buf[..len as usize])))
    }
}

/// The exe's FileDescription (e.g. "Counter-Strike 2"), else its file name.
fn display_name(exe: &Path) -> String {
    let stem = || exe.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "New game".into());
    version_string(exe, "FileDescription")
        .or_else(|| version_string(exe, "ProductName"))
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(stem)
}

fn version_string(exe: &Path, field: &str) -> Option<String> {
    let file = HSTRING::from(exe.as_os_str());
    // SAFETY: `file` is a valid NUL-terminated wide string for the call.
    let size = unsafe { GetFileVersionInfoSizeW(&file, None) };
    if size == 0 {
        return None;
    }
    // A u32 buffer, not u8: the values read out of it below are u16s, and
    // the block's structures are DWORD-aligned relative to its start.
    let mut data = vec![0u32; (size as usize).div_ceil(4)];
    // SAFETY: `data` holds at least `size` bytes, as the call is told.
    unsafe { GetFileVersionInfoW(&file, None, size, data.as_mut_ptr().cast()) }.ok()?;
    let query = |sub: &str| -> Option<(*const u16, u32)> {
        let mut ptr: *mut c_void = std::ptr::null_mut();
        let mut len = 0u32;
        // SAFETY: `data` is the version block GetFileVersionInfoW filled in;
        // the API only writes `ptr` (into `data`) and `len`.
        unsafe { VerQueryValueW(data.as_ptr().cast(), &HSTRING::from(sub), &mut ptr, &mut len) }
            .as_bool()
            .then_some((ptr as *const u16, len))
            .filter(|(p, l)| !p.is_null() && *l > 0)
    };
    // First language/code page pair: two u16s (`len` is in bytes here).
    let (tr, len) = query(r"\VarFileInfo\Translation")?;
    if len < 4 {
        return None;
    }
    // SAFETY: `tr` points at `len` >= 4 bytes inside `data`, which is alive
    // and u16-aligned (see above).
    let (lang, cp) = unsafe { (*tr, *tr.add(1)) };
    // For strings, `chars` counts UTF-16 units, terminator included.
    let (text, chars) = query(&format!(r"\StringFileInfo\{lang:04x}{cp:04x}\{field}"))?;
    // SAFETY: `text` points at `chars` u16s inside `data`, which outlives
    // `wide` and is u16-aligned.
    let wide = unsafe { std::slice::from_raw_parts(text, chars as usize) };
    Some(String::from_utf16_lossy(wide).trim_end_matches('\0').to_owned())
}

fn cache_icon(exe: &Path, png: &Path) -> Result<(), String> {
    let (w, h, rgba) = exe_icon_rgba(exe).ok_or("the exe has no icon")?;
    let mut bytes = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut bytes, w, h);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut writer = enc.write_header().map_err(|e| e.to_string())?;
        writer.write_image_data(&rgba).map_err(|e| e.to_string())?;
    }
    let part = png.with_extension("png.part");
    std::fs::write(&part, bytes).and_then(|()| std::fs::rename(&part, png)).map_err(|e| e.to_string())
}

/// The exe's main icon as top-down RGBA.
fn exe_icon_rgba(exe: &Path) -> Option<(u32, u32, Vec<u8>)> {
    // SAFETY: SHDefExtractIconW only writes `icon`, which is checked before
    // use and destroyed exactly once; `icon_rgba` gets a valid HICON.
    unsafe {
        let mut icon = HICON::default();
        SHDefExtractIconW(&HSTRING::from(exe.as_os_str()), 0, 0, Some(&mut icon), None, ICON_SIZE).ok().ok()?;
        if icon.is_invalid() {
            return None;
        }
        let pixels = icon_rgba(icon);
        let _ = DestroyIcon(icon);
        pixels
    }
}

/// # Safety
///
/// `icon` must be a valid icon handle. It isn't destroyed here; the
/// bitmaps GetIconInfo creates from it are.
unsafe fn icon_rgba(icon: HICON) -> Option<(u32, u32, Vec<u8>)> {
    let mut info = ICONINFO::default();
    // SAFETY: `icon` is valid (the caller's contract); `info` outlives the call.
    unsafe { GetIconInfo(icon, &mut info) }.ok()?;
    let (color, mask) = (info.hbmColor, info.hbmMask);
    let result = (|| {
        let mut bm = BITMAP::default();
        // SAFETY: `bm` is a BITMAP and the call is told its exact size; a
        // null `color` (monochrome icon) makes the call fail and return 0.
        let read =
            unsafe { GetObjectW(HGDIOBJ(color.0), size_of::<BITMAP>() as i32, Some((&mut bm as *mut BITMAP).cast())) };
        if read == 0 {
            return None;
        }
        let (w, h) = (bm.bmWidth.max(0) as u32, bm.bmHeight.max(0) as u32);
        // SAFETY: `color` is live until the DeleteObject below, and `w`×`h`
        // is its size, just read by GetObjectW.
        let mut bgra = unsafe { bitmap_bgra(color, w, h) }?;
        // Old-style icons have no alpha channel: take transparency from the mask.
        if bgra.as_chunks::<4>().0.iter().all(|p| p[3] == 0) {
            // SAFETY: `mask` is live until the DeleteObject below and is at
            // least as big as `color` (taller for monochrome masks).
            let m = unsafe { bitmap_bgra(mask, w, h) }?;
            for (p, m) in bgra.as_chunks_mut::<4>().0.iter_mut().zip(m.as_chunks::<4>().0.iter()) {
                p[3] = if m[0] == 0 { 255 } else { 0 };
            }
        }
        for p in bgra.as_chunks_mut::<4>().0.iter_mut() {
            p.swap(0, 2);
        }
        Some((w, h, bgra))
    })();
    // SAFETY: GetIconInfo created both bitmaps for us; they're deleted once,
    // here, after the last use. Deleting a null handle just fails.
    unsafe {
        let _ = DeleteObject(HGDIOBJ(color.0));
        let _ = DeleteObject(HGDIOBJ(mask.0));
    }
    result
}

/// # Safety
///
/// `bitmap` must be invalid (then `None` is returned) or a live bitmap
/// handle at least `w`×`h` pixels in size.
unsafe fn bitmap_bgra(bitmap: HBITMAP, w: u32, h: u32) -> Option<Vec<u8>> {
    if bitmap.is_invalid() || w == 0 || h == 0 {
        return None;
    }
    let mut bmi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w as i32,
            biHeight: -(h as i32), // top-down
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut buf = vec![0u8; (w * h * 4) as usize];
    // SAFETY: `bitmap` is live and at least `w`×`h` (the caller's contract);
    // `buf` holds `h` rows of `w` 32-bit pixels, as `bmi` describes. The DC
    // is created and deleted here.
    let lines = unsafe {
        let dc = CreateCompatibleDC(None);
        let lines = GetDIBits(dc, bitmap, 0, h, Some(buf.as_mut_ptr().cast()), &mut bmi, DIB_RGB_COLORS);
        let _ = DeleteDC(dc);
        lines
    };
    (lines == h as i32).then_some(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_steam_library_paths() {
        let vdf = r#""libraryfolders"
{
	"0"
	{
		"path"		"C:\\Program Files (x86)\\Steam"
		"label"		""
	}
	"1"
	{
		"path"		"D:\\SteamLibrary"
	}
}"#;
        let paths: Vec<String> = vdf_paths(vdf).collect();
        assert_eq!(paths, [r"C:\Program Files (x86)\Steam", r"D:\SteamLibrary"]);
    }

    #[test]
    fn extracts_an_exe_icon() {
        // Every Windows install has Notepad.
        let exe = PathBuf::from(std::env::var("SystemRoot").unwrap()).join(r"System32\notepad.exe");
        let (w, h, rgba) = exe_icon_rgba(&exe).expect("notepad has an icon");
        assert!(w >= 32 && h >= 32);
        assert_eq!(rgba.len(), (w * h * 4) as usize);
        assert!(rgba.as_chunks::<4>().0.iter().any(|p| p[3] == 255), "not fully transparent");
        assert!(rgba.as_chunks::<4>().0.iter().any(|p| p[3] == 0), "has a transparent background");
        assert!(display_name(&exe).contains("Notepad") || display_name(&exe) == "notepad");
    }
}
