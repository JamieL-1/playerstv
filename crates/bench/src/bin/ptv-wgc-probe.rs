//! Measures how many of the test game's frames Windows Graphics Capture
//! delivers, without ffmpeg in the way, to tell whether frames are lost by
//! Windows or by ffmpeg's `gfxcapture`. Starts the game like a launcher
//! would (no console window), brings it to the front, captures only its
//! window for a while, and reads each captured frame's index strip.
//!
//! `--ffmpeg` runs ffmpeg's `gfxcapture` under the same conditions instead.

use std::collections::BTreeSet;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use clap::Parser;
use ptv_bench::frame_log::FrameLog;
use ptv_bench::pattern::{self, BLOCKS};
use ptv_bench::win::window;
use windows::core::Interface;
use windows::Foundation::{TimeSpan, TypedEventHandler};
use windows::Graphics::Capture::{Direct3D11CaptureFramePool, GraphicsCaptureItem};
use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D, D3D11_BOX, D3D11_CPU_ACCESS_READ,
    D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAPPED_SUBRESOURCE, D3D11_MAP_READ, D3D11_SDK_VERSION,
    D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING,
};
use windows::Win32::Graphics::Dxgi::IDXGIDevice;
use windows::Win32::System::WinRT::Direct3D11::{CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess};
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;
use windows::Win32::System::WinRT::{RoInitialize, RO_INIT_MULTITHREADED};
use windows::Win32::UI::WindowsAndMessaging::FindWindowW;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
/// Frames between copying a frame's strip and reading it back, so reading
/// never waits for the GPU.
const READBACK_LAG: usize = 4;

#[derive(Parser)]
struct Args {
    #[arg(long, default_value_t = 10)]
    seconds: u64,
    #[arg(long, default_value_t = 60)]
    fps: u32,
    /// Buffers in the capture frame pool (ffmpeg's gfxcapture uses 2).
    #[arg(long, default_value_t = 2)]
    buffers: i32,
    /// The session's minimum update interval in ms; 0 leaves the default.
    #[arg(long, default_value_t = 0.0)]
    min_interval_ms: f64,
    /// Run ffmpeg's gfxcapture instead of our own capture.
    #[arg(long)]
    ffmpeg: bool,
    /// Run the game in a window of this size instead of borderless.
    #[arg(long)]
    windowed: Option<String>,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let dir = std::env::temp_dir().join(format!("ptv-wgc-probe-{}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    let log_path = dir.join("frames.csv");

    // Started without a console, as a launcher starts a game: an extra
    // console window on the monitor changes how Windows presents the game.
    let me = std::env::current_exe()?;
    let game_exe = me.with_file_name("ptv-bench-game.exe");
    let mut game = Command::new(&game_exe);
    game.args(["--fps", &args.fps.to_string(), "--seconds", &(args.seconds + 8).to_string(), "--log"])
        .arg(&log_path)
        .creation_flags(CREATE_NO_WINDOW);
    if let Some(size) = &args.windowed {
        game.args(["--windowed", size]);
    }
    let mut game = game.spawn().with_context(|| format!("starting {}", game_exe.display()))?;
    window::focus_game(Duration::from_secs(10))?;
    std::thread::sleep(Duration::from_secs(2));

    let captured = if args.ffmpeg { None } else { Some(capture(&args)?) };
    let clip = dir.join("clip.mp4");
    if args.ffmpeg {
        run_ffmpeg(&args, &clip)?;
    }
    let focused = window::game_has_focus();
    game.wait()?;
    let log = FrameLog::read(&log_path)?;

    match captured {
        Some(c) => {
            let label = format!("own WGC, {} buffers, min interval {} ms", args.buffers, args.min_interval_ms);
            summarise(&label, c.delivered, &c.indices, &log, focused);
        }
        None => {
            let r = ptv_bench::analyze::analyze(&ffmpeg_exe(), &clip, log.pattern, Some(&log.frames))?;
            let drawn = r.frames as u64 - r.unreadable as u64 + r.dropped;
            println!(
                "ffmpeg gfxcapture: kept {} of {drawn} drawn ({:.1}%), longest gap {}{}",
                r.frames - r.unreadable,
                (r.frames - r.unreadable) as f64 / drawn.max(1) as f64 * 100.0,
                r.longest_gap,
                if focused { "" } else { " [game lost focus]" }
            );
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}

fn summarise(label: &str, delivered: usize, indices: &[Option<u64>], log: &FrameLog, focused: bool) {
    let read: BTreeSet<u64> = indices.iter().flatten().copied().collect();
    let (Some(&first), Some(&last)) = (read.first(), read.last()) else {
        println!("{label}: no readable frames ({delivered} delivered)");
        return;
    };
    let drawn: BTreeSet<u64> = log.frames.iter().map(|f| f.index).filter(|i| (first..=last).contains(i)).collect();
    let kept = drawn.intersection(&read).count();
    println!(
        "{label}: {delivered} frames delivered, {} distinct, {} unreadable; kept {kept} of {} drawn ({:.1}%){}",
        read.len(),
        indices.iter().filter(|i| i.is_none()).count(),
        drawn.len(),
        kept as f64 / drawn.len().max(1) as f64 * 100.0,
        if focused { "" } else { " [game lost focus]" }
    );
}

struct Captured {
    delivered: usize,
    indices: Vec<Option<u64>>,
}

/// Captures the game's window for `args.seconds` and reads every frame's
/// index strip.
fn capture(args: &Args) -> Result<Captured> {
    // SAFETY: plain FFI calls. RoInitialize may report the apartment is
    // already initialised, which is fine. The out-pointers are valid locals
    // and the class name is a 'static wide string.
    let (device, context, hwnd) = unsafe {
        let _ = RoInitialize(RO_INIT_MULTITHREADED);
        let mut device: Option<ID3D11Device> = None;
        let mut context: Option<ID3D11DeviceContext> = None;
        D3D11CreateDevice(
            None,
            D3D_DRIVER_TYPE_HARDWARE,
            Default::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            None,
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            Some(&mut context),
        )?;
        let hwnd = FindWindowW(windows::core::w!("ptv-bench-game"), None).context("no game window")?;
        (device.context("no device")?, context.context("no context")?, hwnd)
    };
    let item = capture_item(hwnd)?;
    let size = item.Size()?;
    // SAFETY: CreateDirect3D11DeviceFromDXGIDevice takes any DXGI device;
    // ours stays alive for the whole capture.
    let winrt_device: IDirect3DDevice =
        unsafe { CreateDirect3D11DeviceFromDXGIDevice(&device.cast::<IDXGIDevice>()?)? }.cast()?;
    let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
        &winrt_device,
        DirectXPixelFormat::B8G8R8A8UIntNormalized,
        args.buffers,
        size,
    )?;
    let session = pool.CreateCaptureSession(&item)?;
    let _ = session.SetIsBorderRequired(false);
    let _ = session.SetIsCursorCaptureEnabled(false);
    if args.min_interval_ms > 0.0 {
        session.SetMinUpdateInterval(TimeSpan { Duration: (args.min_interval_ms * 10_000.0) as i64 })?;
    }

    // The handler only wakes this thread, which does the reading.
    let (tx, rx) = crossbeam_channel::unbounded::<()>();
    pool.FrameArrived(&TypedEventHandler::new(move |_, _| {
        let _ = tx.send(());
        Ok(())
    }))?;

    let width = size.Width as u32;
    let height = size.Height as u32;
    let row = pattern::Pattern::new(width.max(BLOCKS as u32 * 4), height.max(64), args.fps).strip_height() / 2;
    let staging: Vec<ID3D11Texture2D> =
        (0..READBACK_LAG + 1).map(|_| staging_row(&device, width)).collect::<Result<_>>()?;
    let mut indices = Vec::new();
    let mut delivered = 0usize;

    session.StartCapture()?;
    let end = Instant::now() + Duration::from_secs(args.seconds);
    while Instant::now() < end {
        if rx.recv_timeout(Duration::from_millis(50)).is_err() {
            continue;
        }
        while let Ok(frame) = pool.TryGetNextFrame() {
            let surface = frame.Surface()?;
            // SAFETY: the surface wraps a D3D11 texture on our device, which
            // the frame keeps alive until it's dropped below; the copy box
            // lies inside both textures.
            unsafe {
                let texture: ID3D11Texture2D = surface.cast::<IDirect3DDxgiInterfaceAccess>()?.GetInterface()?;
                let slot = delivered % staging.len();
                let area = D3D11_BOX { left: 0, top: row, front: 0, right: width, bottom: row + 1, back: 1 };
                context.CopySubresourceRegion(&staging[slot], 0, 0, 0, 0, &texture, 0, Some(&area));
            }
            drop(frame);
            delivered += 1;
            if delivered > READBACK_LAG {
                indices.push(read_index(&context, &staging[(delivered - 1 - READBACK_LAG) % staging.len()], width)?);
            }
        }
    }
    session.Close()?;
    pool.Close()?;
    for n in delivered.saturating_sub(READBACK_LAG)..delivered {
        indices.push(read_index(&context, &staging[n % staging.len()], width)?);
    }
    Ok(Captured { delivered, indices })
}

fn capture_item(hwnd: HWND) -> Result<GraphicsCaptureItem> {
    let interop = windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()?;
    // SAFETY: `hwnd` is the game's live top-level window.
    Ok(unsafe { interop.CreateForWindow(hwnd)? })
}

/// A one-row CPU-readable texture `width` wide.
fn staging_row(device: &ID3D11Device, width: u32) -> Result<ID3D11Texture2D> {
    let desc = D3D11_TEXTURE2D_DESC {
        Width: width,
        Height: 1,
        MipLevels: 1,
        ArraySize: 1,
        Format: windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM,
        SampleDesc: windows::Win32::Graphics::Dxgi::Common::DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        Usage: D3D11_USAGE_STAGING,
        CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
        ..Default::default()
    };
    let mut texture = None;
    // SAFETY: `desc` and the out-pointer are valid for the call.
    unsafe { device.CreateTexture2D(&desc, None, Some(&mut texture))? };
    texture.context("CreateTexture2D returned nothing")
}

/// Decodes the frame index from a copied strip row.
fn read_index(context: &ID3D11DeviceContext, row: &ID3D11Texture2D, width: u32) -> Result<Option<u64>> {
    let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
    // SAFETY: `row` is a staging texture we created with CPU read access;
    // while mapped, `pData` points at `width` BGRA pixels, and it's
    // unmapped before returning.
    unsafe {
        context.Map(row, 0, D3D11_MAP_READ, 0, Some(&mut mapped))?;
        let pixels = std::slice::from_raw_parts(mapped.pData as *const u8, width as usize * 4);
        let block = width as usize / BLOCKS;
        let blocks: [u8; BLOCKS] = std::array::from_fn(|b| {
            let px = &pixels[b * block * 4..(b + 1) * block * 4];
            (px.chunks(4).map(|p| p[1] as usize).sum::<usize>() / block) as u8
        });
        context.Unmap(row, 0);
        Ok(pattern::decode(&blocks))
    }
}

fn ffmpeg_exe() -> PathBuf {
    ptv_common::paths::ffmpeg_exe()
}

/// Records the game window with ffmpeg's gfxcapture, keeping capture times,
/// on the fastest preset so the encoder doesn't hold capture back.
fn run_ffmpeg(args: &Args, clip: &Path) -> Result<()> {
    // SAFETY: plain FFI call with a 'static class name.
    let hwnd = unsafe { FindWindowW(windows::core::w!("ptv-bench-game"), None) }.context("no game window")?;
    let status = Command::new(ffmpeg_exe())
        .args(["-hide_banner", "-nostats", "-loglevel", "error", "-y", "-filter_complex"])
        .arg(format!("gfxcapture=hwnd={}:max_framerate={}:capture_cursor=0[v]", hwnd.0 as usize, args.fps * 2))
        .args(["-map", "[v]", "-t", &args.seconds.to_string(), "-fps_mode", "vfr", "-enc_time_base", "1/90000"])
        .args(["-c:v", "h264_nvenc", "-preset", "p1", "-cq", "19"])
        .arg(clip)
        .creation_flags(CREATE_NO_WINDOW)
        .status()?;
    if !status.success() {
        bail!("ffmpeg exited with {status}");
    }
    Ok(())
}
