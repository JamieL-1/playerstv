//! Creating the main window (in code rather than tauri.conf.json, so tests
//! can open it without disturbing the person at the computer).

use tauri::webview::Color;
use tauri::{App, WebviewUrl, WebviewWindowBuilder};

/// Discord's dark main-panel grey, so the window never flashes white.
const BACKGROUND: Color = Color(0x31, 0x33, 0x38, 0xff);

pub fn create_main(app: &App) -> tauri::Result<()> {
    let builder = WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
        .title("playerstv")
        .inner_size(1280.0, 800.0)
        .min_inner_size(940.0, 560.0)
        .background_color(BACKGROUND);

    match test_mode() {
        Some(test) => {
            // Off-screen and unfocused: screenshots for UI tests without
            // popping over (or stealing focus from) whatever is running.
            // Created hidden and moved after creation, because a position
            // given at creation time gets pulled back onto the screen.
            let builder = builder
                .visible(false)
                .focused(false)
                .skip_taskbar(true)
                // WebView2 stops painting windows it thinks are hidden, which
                // off-screen counts as; keep rendering so captures get frames.
                .additional_browser_args(
                    "--disable-features=CalculateNativeWinOcclusion --disable-backgrounding-occluded-windows --disable-renderer-backgrounding",
                );
            let builder = match &test.route {
                // main.tsx reads this before creating the router.
                Some(route) => builder.initialization_script(format!("window.__PTV_TEST_ROUTE__ = '{route}';")),
                None => builder,
            };
            let builder = match &test.eval {
                // Inlined rather than eval'd: the CSP blocks eval, but not
                // initialization scripts.
                Some(js) => builder.initialization_script(format!(
                    "window.addEventListener('load', () => setTimeout(() => {{\n{js}\n}}, 500));"
                )),
                None => builder,
            };
            let window = builder.build()?;
            window.set_position(tauri::PhysicalPosition::new(-6000, -6000))?;
            show_without_activating(&window);
            eprintln!("playerstv-ui test mode: route={:?} position={:?}", test.route, window.outer_position().ok());
            if let Some(path) = test.screenshot {
                screenshot_then_exit(app.handle().clone(), window, path, test.settle);
            }
        }
        None => {
            builder.center().build()?;
        }
    }
    Ok(())
}

/// Shows a window without giving it focus (SW_SHOWNOACTIVATE).
fn show_without_activating(window: &tauri::WebviewWindow) {
    use windows::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_SHOWNOACTIVATE};
    if let Ok(hwnd) = window.hwnd() {
        unsafe {
            let _ = ShowWindow(windows::Win32::Foundation::HWND(hwnd.0), SW_SHOWNOACTIVATE);
        }
    }
}

struct TestMode {
    route: Option<String>,
    screenshot: Option<std::path::PathBuf>,
    /// JavaScript to run once the page has loaded (to drive the UI).
    eval: Option<String>,
    settle: std::time::Duration,
}

/// After the page settles, renders it to a PNG through WebView2 itself
/// (works while the window is off-screen, unlike screen capture) and exits.
#[cfg(debug_assertions)]
fn screenshot_then_exit(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    path: std::path::PathBuf,
    settle: std::time::Duration,
) {
    std::thread::spawn(move || {
        std::thread::sleep(settle);
        let done = app.clone();
        let result = window.with_webview(move |wv| {
            use webview2_com::CapturePreviewCompletedHandler;
            use webview2_com::Microsoft::Web::WebView2::Win32::COREWEBVIEW2_CAPTURE_PREVIEW_IMAGE_FORMAT_PNG;
            use windows::core::HSTRING;
            use windows::Win32::Storage::FileSystem::FILE_ATTRIBUTE_NORMAL;
            use windows::Win32::System::Com::{STGM_CREATE, STGM_WRITE};
            use windows::Win32::UI::Shell::SHCreateStreamOnFileEx;

            let capture = || -> windows::core::Result<()> {
                unsafe {
                    let core = wv.controller().CoreWebView2()?;
                    let stream = SHCreateStreamOnFileEx(
                        &HSTRING::from(path.as_os_str()),
                        (STGM_CREATE | STGM_WRITE).0,
                        FILE_ATTRIBUTE_NORMAL.0,
                        true,
                        None,
                    )?;
                    let app = done.clone();
                    // COM clone (AddRef): the handler keeps the stream alive
                    // until the capture has been written.
                    let keep = stream.clone();
                    let handler = CapturePreviewCompletedHandler::create(Box::new(move |hr| {
                        drop(keep); // flush the file before exiting
                        eprintln!("playerstv-ui screenshot: {hr:?}");
                        app.exit(0);
                        Ok(())
                    }));
                    core.CapturePreview(COREWEBVIEW2_CAPTURE_PREVIEW_IMAGE_FORMAT_PNG, &stream, &handler)
                }
            };
            if let Err(e) = capture() {
                eprintln!("playerstv-ui screenshot failed: {e}");
                done.exit(1);
            }
        });
        if let Err(e) = result {
            eprintln!("playerstv-ui screenshot failed: {e}");
            app.exit(1);
        }
    });
}

#[cfg(not(debug_assertions))]
fn screenshot_then_exit(_: tauri::AppHandle, _: tauri::WebviewWindow, _: std::path::PathBuf, _: std::time::Duration) {}

/// `PLAYERSTV_UI_TEST=1` (debug builds only), with optional
/// `PLAYERSTV_UI_ROUTE=/settings/video` to start on a page and
/// `PLAYERSTV_UI_SCREENSHOT=out.png` to save a screenshot and exit
/// (`PLAYERSTV_UI_SETTLE_MS`, default 2500, after load), and
/// `PLAYERSTV_UI_EVAL=<js>` to run a script half a second after load.
#[cfg(debug_assertions)]
fn test_mode() -> Option<TestMode> {
    std::env::var_os("PLAYERSTV_UI_TEST")?;
    let screenshot = std::env::var_os("PLAYERSTV_UI_SCREENSHOT").map(std::path::PathBuf::from);
    let route = std::env::var("PLAYERSTV_UI_ROUTE")
        .ok()
        // "settings/video" or "/settings/video" (some shells rewrite a
        // leading slash into a filesystem path).
        .map(|r| format!("/{}", r.trim_start_matches('/')))
        // Only plain route characters reach the injected script.
        .filter(|r| r.chars().all(|c| c.is_ascii_alphanumeric() || "/-_?=&%.".contains(c)));
    let eval = std::env::var("PLAYERSTV_UI_EVAL").ok();
    let settle = std::env::var("PLAYERSTV_UI_SETTLE_MS").ok().and_then(|s| s.parse().ok()).unwrap_or(2500);
    Some(TestMode { route, screenshot, eval, settle: std::time::Duration::from_millis(settle) })
}

#[cfg(not(debug_assertions))]
fn test_mode() -> Option<TestMode> {
    None
}
