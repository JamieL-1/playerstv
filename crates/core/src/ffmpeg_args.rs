//! Builds the long-running capture/encode ffmpeg command.
//!
//! Video never leaves the GPU: `gfxcapture` (Windows Graphics Capture) yields
//! D3D11 textures, scales them itself, and NVENC encodes them directly. Audio
//! arrives as raw f32 PCM over named pipes fed by the capture crate.
//!
//! Frames keep the time Windows captured them at (variable frame rate).
//! Forcing them onto a fixed 1/fps grid used to lose about a third of a
//! 60 fps game's frames: one that landed a little late left its slot to a
//! repeat of the previous frame, then shared the next slot with its
//! successor and was dropped.

use std::path::Path;

use crate::config::{Codec, Resolution, VideoConfig};
use crate::ring::{Container, SEGMENT_SECONDS};

pub const AUDIO_RATE: u32 = 48_000;
pub const AUDIO_CHANNELS: u32 = 2;

/// The window being captured.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CaptureTarget {
    pub hwnd: u64,
    /// Current client-area size, used to keep the aspect ratio when scaling.
    pub width: u32,
    pub height: u32,
}

/// One raw-PCM audio input delivered on a named pipe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioInput {
    pub pipe_path: String,
    pub title: String,
    /// Level of this input in the mixed track, in percent.
    pub mix_volume: u32,
}

#[derive(Debug, Clone)]
pub struct RecordSpec<'a> {
    pub target: CaptureTarget,
    pub video: &'a VideoConfig,
    pub audio: &'a [AudioInput],
    /// Add a first audio track mixing all inputs (only with 2+ inputs).
    pub mixed_track: bool,
    pub audio_bitrate_kbps: u32,
    pub segment_dir: &'a Path,
    pub start_index: u64,
    /// Wall-clock time, in microseconds since the Unix epoch, of the audio
    /// inputs' first sample. Video is stamped on the same clock so the two
    /// line up however long each takes to start.
    pub audio_origin_unix_us: u64,
}

pub const MIXED_TRACK_TITLE: &str = "Mixed";

/// Windows Graphics Capture may deliver frames this many times faster than
/// the target rate, and [`pace_filter`] thins them out. Capping capture at
/// exactly the target rate made Windows skip frames that arrived a little
/// early, as frames of a 60 fps game shown on a faster monitor often do.
const CAPTURE_HEADROOM: u32 = 2;

/// Passes at most `fps` frames a second on average, without needing them
/// evenly spaced: each second earns `fps` credits, a frame costs one, and
/// one may arrive up to a quarter of a credit early. Unused credit is capped
/// at two frames, so a game that slows down and recovers can't burst.
fn pace_filter(fps: u32) -> String {
    format!("select='st(0,if(n,min(2,ld(0)+(t-ld(1))*{fps}),1));st(1,t);if(gte(ld(0),0.75),st(0,ld(0)-1)*0+1,0)'")
}

/// Moves video timestamps onto the audio's clock: the first frame is placed
/// at its wall-clock time after `origin_unix_us`, and the rest follow at
/// their capture times. gfxcapture alone starts video at zero on its first
/// frame, while audio starts at zero when its feeder does, a varying
/// 10 to 110 ms apart.
fn align_filter(origin_unix_us: u64) -> String {
    format!("setpts='PTS+if(eq(N,0),st(0,(RTCTIME-{origin_unix_us})/1000000),ld(0))/TB'")
}

fn has_mix(spec: &RecordSpec) -> bool {
    spec.mixed_track && spec.audio.len() >= 2
}

/// Titles of the audio tracks `record_args` produces, in stream order.
pub fn track_titles(spec: &RecordSpec) -> Vec<String> {
    let mix = has_mix(spec).then(|| MIXED_TRACK_TITLE.to_owned());
    mix.into_iter().chain(spec.audio.iter().map(|a| a.title.clone())).collect()
}

/// `[0:a]volume=1.00[m0];[1:a]volume=1.50[m1];[m0][m1]amix=…[mix]`
fn mix_graph(audio: &[AudioInput]) -> String {
    let mut g = String::new();
    for (i, input) in audio.iter().enumerate() {
        g.push_str(&format!("[{i}:a]volume={:.2}[m{i}];", input.mix_volume as f64 / 100.0));
    }
    for i in 0..audio.len() {
        g.push_str(&format!("[m{i}]"));
    }
    // normalize=0 keeps each source at its own level instead of dividing by
    // the input count; the limiter catches the peaks that summing can cause.
    g.push_str(&format!(
        "amix=inputs={}:normalize=0:dropout_transition=0,alimiter=limit=0.97:level=false[mix]",
        audio.len()
    ));
    g
}

/// Output frame size as `(width, height)` gfxcapture options. Negative values
/// tell gfxcapture to round the native size down to a multiple of 2.
pub fn output_size(target: CaptureTarget, video: &VideoConfig) -> (i64, i64) {
    let known = target.width > 0 && target.height > 0;
    let max = video.codec.max_dimension();
    let fits = |w: u32, h: u32| w <= max && h <= max;
    let native_even = (target.width / 2 * 2, target.height / 2 * 2);
    match (video.resolution, video.resolution.height()) {
        // 2× only makes sense as an exact integer scale; if the encoder
        // can't take it, record native instead.
        (Resolution::Double, _) if known && fits(native_even.0 * 2, native_even.1 * 2) => {
            ((native_even.0 * 2) as i64, (native_even.1 * 2) as i64)
        }
        (_, None) if !known || fits(native_even.0, native_even.1) => (-2, -2),
        // Native window bigger than the encoder allows (e.g. 5120×1440 with H.264).
        (_, None) => fit_within(native_even.0 as f64 / native_even.1 as f64, native_even.1, max),
        (_, Some(h)) => {
            let aspect = if known { target.width as f64 / target.height as f64 } else { 16.0 / 9.0 };
            fit_within(aspect, h, max)
        }
    }
}

/// The largest even size with this aspect ratio, `height` tall at most,
/// with neither side over `max`.
fn fit_within(aspect: f64, height: u32, max: u32) -> (i64, i64) {
    let mut h = height.min(max) as f64;
    if h * aspect > max as f64 {
        h = max as f64 / aspect;
    }
    let even = |x: f64| ((x / 2.0).floor() as i64 * 2).max(2);
    (even(h * aspect).min(max as i64), even(h))
}

/// The frame size that will actually be encoded (native resolves to the
/// window size; an unknown/minimised window is assumed to be 1080p).
pub fn encoded_size(target: CaptureTarget, video: &VideoConfig) -> (u32, u32) {
    match output_size(target, video) {
        (w, h) if w > 0 && h > 0 => (w as u32, h as u32),
        _ if target.width > 1 && target.height > 1 => (target.width / 2 * 2, target.height / 2 * 2),
        _ => (1920, 1080),
    }
}

/// Above this many pixels per second, the slower high-quality encoder mode
/// (multipass) can't keep up even on an RTX 5080: measured ~87 fps
/// for 4K120 HEVC with them vs ~111 fps with split-frame encoding.
const HEAVY_PIXEL_RATE: u64 = 520_000_000;

/// How the encoder is configured for a given output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncodePlan {
    pub width: u32,
    pub height: u32,
    pub preset: &'static str,
    /// Multipass (quarter-res).
    pub quality_modes: bool,
    /// Split each frame across multiple NVENC engines (HEVC/AV1 only).
    pub split_frames: bool,
    pub cq: u32,
    pub cap_mbps: u32,
}

pub fn encode_plan(target: CaptureTarget, v: &VideoConfig) -> EncodePlan {
    let (width, height) = encoded_size(target, v);
    let pixels = width as u64 * height as u64;
    let heavy = pixels * v.fps as u64 > HEAVY_PIXEL_RATE;
    let auto_cap = {
        let bits = pixels as f64 * v.fps as f64 * v.quality.cap_bits_per_pixel();
        ((bits / 1e6).round() as u32).clamp(4, 300)
    };
    EncodePlan {
        width,
        height,
        preset: if heavy { "p4" } else { "p5" },
        quality_modes: !heavy,
        split_frames: heavy && v.codec != Codec::H264,
        cq: v.quality.cq(v.codec),
        cap_mbps: v.bitrate_cap_mbps.unwrap_or(auto_cap),
    }
}

/// Human-readable summary, e.g. "1900×1030 · 120 fps · H.264 · High quality".
pub fn describe(plan: &EncodePlan, v: &VideoConfig) -> String {
    let codec = match v.codec {
        Codec::H264 => "H.264",
        Codec::Hevc => "HEVC",
        Codec::Av1 => "AV1",
    };
    format!("{}×{} · {} fps · {codec} · {} quality", plan.width, plan.height, v.fps, v.quality.short_name())
}

#[rustfmt::skip] // keep each ffmpeg flag next to its value
pub fn record_args(spec: &RecordSpec) -> Vec<String> {
    let v = spec.video;
    let (w, h) = output_size(spec.target, v);
    let mut a: Vec<String> = Vec::new();
    let mut push = |items: &[&str]| a.extend(items.iter().map(|s| s.to_string()));

    push(&["-hide_banner", "-nostats", "-loglevel", "warning", "-y"]);

    // Point sampling keeps 2× pixel-perfect; bicubic beats the default
    // bilinear for every other rescale.
    let scale_mode = if v.resolution == Resolution::Double { "point" } else { "bicubic" };
    let mut graph = format!(
        "gfxcapture=hwnd={hwnd}:max_framerate={rate}:resize_mode=scale_aspect:scale_mode={scale_mode}:width={w}:height={h}:capture_cursor=1",
        hwnd = spec.target.hwnd,
        rate = v.fps * CAPTURE_HEADROOM,
    );
    if !spec.audio.is_empty() {
        graph.push(',');
        graph.push_str(&align_filter(spec.audio_origin_unix_us));
    }
    graph.push(',');
    graph.push_str(&pace_filter(v.fps));
    graph.push_str("[v]");
    let mix = has_mix(spec);
    if mix {
        graph.push(';');
        graph.push_str(&mix_graph(spec.audio));
    }
    push(&["-filter_complex", &graph]);

    for input in spec.audio {
        push(&[
            "-thread_queue_size", "1024",
            "-f", "f32le",
            "-ar", &AUDIO_RATE.to_string(),
            "-ac", &AUDIO_CHANNELS.to_string(),
            "-i", &input.pipe_path,
        ]);
    }

    push(&["-map", "[v]"]);
    if mix {
        push(&["-map", "[mix]"]);
    }
    for i in 0..spec.audio.len() {
        push(&["-map", &format!("{i}:a")]);
    }

    let plan = encode_plan(spec.target, v);
    let gop = (v.fps * SEGMENT_SECONDS).to_string();
    // NVENC budgets bits per frame from the frame rate it's told, which is
    // the capture rate, CAPTURE_HEADROOM times the rate frames really
    // arrive at. Scaling the cap by the same factor keeps it per second.
    let cap_mbps = plan.cap_mbps * CAPTURE_HEADROOM;
    push(&[
        "-c:v", v.codec.nvenc_encoder(),
        "-preset", plan.preset,
        "-tune", "hq",
        "-rc", "vbr",
        "-cq", &plan.cq.to_string(),
        "-b:v", "0",
        "-maxrate", &format!("{cap_mbps}M"),
        "-bufsize", &format!("{}M", cap_mbps * 2),
        // Adaptive quantisation spends bits on detail (text, UI edges) rather
        // than flat areas; B-frames as references are nearly free quality.
        "-spatial-aq", "1",
        "-temporal-aq", "1",
        "-bf", "3",
        "-b_ref_mode", "middle",
        // A keyframe every segment by time, since frames needn't come at
        // the full rate; -g only caps the run of frames between them.
        "-g", &gop,
        "-keyint_min", &gop,
        "-force_key_frames", &format!("expr:gte(t,n_forced*{SEGMENT_SECONDS})"),
        "-forced-idr", "1",
        // NVENC's default surface pool costs ~100 MB for nothing at these
        // frame rates; it raises the count itself when B-frames need more.
        // No output delay means clips include the newest frames. It also
        // turns rate-control lookahead off, so that isn't asked for: it
        // changed nothing in the output but cost ~95 MB of RAM.
        "-surfaces", "8",
        "-delay", "0",
        // Keep each frame's capture time. The segment muxer doesn't say it
        // allows a variable rate, so ffmpeg would otherwise force 1/fps
        // slots; 1/90000 is MPEG-TS's own clock.
        "-fps_mode", "vfr",
        "-enc_time_base", "1/90000",
    ]);
    if plan.quality_modes {
        push(&["-multipass", "qres"]);
    }
    if plan.split_frames {
        push(&["-split_encode_mode", "forced"]);
    }

    if !spec.audio.is_empty() {
        push(&["-c:a", "aac", "-b:a", &format!("{}k", spec.audio_bitrate_kbps)]);
    }

    let container = Container::for_codec(v.codec);
    let pattern = spec.segment_dir.join(container.pattern());
    push(&[
        "-f", "segment",
        "-segment_time", &SEGMENT_SECONDS.to_string(),
        "-segment_format", container.muxer(),
        "-segment_start_number", &spec.start_index.to_string(),
        "-reset_timestamps", if container.reset_timestamps() { "1" } else { "0" },
        &pattern.to_string_lossy(),
    ]);
    a
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Quality;
    use std::path::PathBuf;

    fn target() -> CaptureTarget {
        CaptureTarget { hwnd: 0x1234, width: 1900, height: 1030 }
    }

    #[test]
    fn native_size_rounds_in_ffmpeg() {
        assert_eq!(output_size(target(), &VideoConfig::default()), (-2, -2));
    }

    #[test]
    fn scaled_size_keeps_aspect_and_is_even() {
        let mut v = VideoConfig { resolution: Resolution::P1080, ..Default::default() };
        assert_eq!(output_size(target(), &v), (1992, 1080));
        v.resolution = Resolution::P2160;
        let t = CaptureTarget { width: 2560, height: 1440, ..target() };
        assert_eq!(output_size(t, &v), (3840, 2160));
        // Minimised window reports 0x0: fall back to 16:9.
        let t = CaptureTarget { width: 0, height: 0, ..target() };
        v.resolution = Resolution::P720;
        assert_eq!(output_size(t, &v), (1280, 720));
    }

    fn game_and_mic() -> Vec<AudioInput> {
        vec![
            AudioInput { pipe_path: r"\\.\pipe\ptv-game".into(), title: "Game".into(), mix_volume: 150 },
            AudioInput { pipe_path: r"\\.\pipe\ptv-mic".into(), title: "Mic".into(), mix_volume: 80 },
        ]
    }

    #[test]
    fn full_command() {
        let v = VideoConfig { codec: Codec::Hevc, fps: 120, ..Default::default() };
        let audio = game_and_mic();
        let dir = PathBuf::from(r"C:\buf");
        let spec = RecordSpec {
            target: target(),
            video: &v,
            audio: &audio,
            mixed_track: true,
            audio_bitrate_kbps: 160,
            segment_dir: &dir,
            start_index: 7,
            audio_origin_unix_us: 1_793_000_000_000_000,
        };
        let args = record_args(&spec);
        let s = args.join(" ");
        // Capture at twice the rate, timed on the audio's clock, then paced
        // down to 120 fps.
        assert!(s.contains(
            "gfxcapture=hwnd=4660:max_framerate=240:resize_mode=scale_aspect:scale_mode=bicubic:width=-2:height=-2:capture_cursor=1,\
             setpts='PTS+if(eq(N,0),st(0,(RTCTIME-1793000000000000)/1000000),ld(0))/TB',\
             select='st(0,if(n,min(2,ld(0)+(t-ld(1))*120),1));st(1,t);if(gte(ld(0),0.75),st(0,ld(0)-1)*0+1,0)'[v]"
        ));
        assert!(s.contains(
            "[v];[0:a]volume=1.50[m0];[1:a]volume=0.80[m1];[m0][m1]amix=inputs=2:normalize=0:dropout_transition=0,alimiter=limit=0.97:level=false[mix]"
        ));
        assert!(s.contains(r"-f f32le -ar 48000 -ac 2 -i \\.\pipe\ptv-game"));
        assert!(s.contains("-map [v] -map [mix] -map 0:a -map 1:a"));
        assert_eq!(track_titles(&spec), ["Mixed", "Game", "Mic"]);
        assert!(s.contains("-c:v hevc_nvenc -preset p5 -tune hq -rc vbr -cq 22 -b:v 0"));
        assert!(s.contains("-spatial-aq 1 -temporal-aq 1 -bf 3 -b_ref_mode middle"));
        assert!(s.contains("-multipass qres"));
        // -delay 0 disables lookahead, so asking for it only costs memory.
        assert!(!s.contains("lookahead"));
        assert!(!s.contains("rgb_mode"));
        assert!(s.contains("-g 240"));
        assert!(s.contains("-force_key_frames expr:gte(t,n_forced*2)"));
        assert!(s.contains("-surfaces 8 -delay 0 -fps_mode vfr -enc_time_base 1/90000"));
        // The cap is doubled to match the doubled rate NVENC is told.
        let cap = encode_plan(target(), &v).cap_mbps;
        assert!(s.contains(&format!("-maxrate {}M -bufsize {}M", cap * 2, cap * 4)));
        assert!(s.contains("-c:a aac -b:a 160k"));
        assert!(s.contains("-segment_start_number 7"));
        assert!(args.last().unwrap().ends_with("seg_%06d.ts"));
    }

    #[test]
    fn double_is_exact_pixel_doubling() {
        let v = VideoConfig { resolution: Resolution::Double, ..Default::default() };
        assert_eq!(output_size(target(), &v), (3800, 2060));
        // Odd window sizes round down to even first so pixels stay square blocks.
        let t = CaptureTarget { width: 1901, height: 1031, ..target() };
        assert_eq!(output_size(t, &v), (3800, 2060));
        let dir = PathBuf::from("buf");
        let s = record_args(&RecordSpec {
            target: target(),
            video: &v,
            audio: &[],
            mixed_track: true,
            audio_bitrate_kbps: 160,
            segment_dir: &dir,
            start_index: 0,
            audio_origin_unix_us: 0,
        })
        .join(" ");
        assert!(s.contains("scale_mode=point:width=3800:height=2060"));
    }

    #[test]
    fn sizes_never_exceed_encoder_limits() {
        let qhd = CaptureTarget { width: 2560, height: 1440, ..target() };
        // 2× of 1440p is 5120×2880: too big for H.264 (4096) → native.
        let v = VideoConfig { resolution: Resolution::Double, ..Default::default() };
        assert_eq!(output_size(qhd, &v), (-2, -2));
        assert_eq!(encoded_size(qhd, &v), (2560, 1440));
        // HEVC goes up to 8192, so 2× is fine there.
        let v = VideoConfig { codec: Codec::Hevc, ..v };
        assert_eq!(output_size(qhd, &v), (5120, 2880));
        // 4K on a 21:9 window would be 5040 wide: scaled to fit H.264.
        let ultrawide = CaptureTarget { width: 3440, height: 1440, ..target() };
        let v = VideoConfig { resolution: Resolution::P2160, ..Default::default() };
        let (w, h) = output_size(ultrawide, &v);
        assert!(w <= 4096 && h <= 4096 && w % 2 == 0 && h % 2 == 0, "{w}x{h}");
        assert_eq!(w, 4096);
        // A native window wider than 4096 is scaled down instead of failing.
        let super_wide = CaptureTarget { width: 5120, height: 1440, ..target() };
        let (w, h) = output_size(super_wide, &VideoConfig::default());
        assert_eq!((w, h), (4096, 1152));
    }

    #[test]
    fn encode_plan_scales_with_pixel_rate() {
        // Native 1900x1030@120 ≈ 235 Mpx/s: full quality modes.
        let v = VideoConfig { fps: 120, ..Default::default() };
        let p = encode_plan(target(), &v);
        assert_eq!((p.width, p.height, p.preset, p.quality_modes, p.split_frames), (1900, 1030, "p5", true, false));
        // 4K120 ≈ 1 Gpx/s: lighter preset, split across NVENCs (not H.264).
        let v = VideoConfig { fps: 120, resolution: Resolution::P2160, ..Default::default() };
        let p = encode_plan(target(), &v);
        assert_eq!((p.preset, p.quality_modes, p.split_frames), ("p4", false, false));
        let v = VideoConfig { codec: Codec::Hevc, ..v };
        assert!(encode_plan(target(), &v).split_frames);
    }

    #[test]
    fn bitrate_cap_and_summary() {
        let v = VideoConfig { fps: 60, quality: Quality::High, ..Default::default() };
        // 1900*1030*60*0.12 ≈ 14 Mbps
        let p = encode_plan(target(), &v);
        assert_eq!(p.cap_mbps, 14);
        assert_eq!(describe(&p, &v), "1900×1030 · 60 fps · H.264 · High quality");
        // Manual cap wins.
        let manual = VideoConfig { bitrate_cap_mbps: Some(50), ..v };
        assert_eq!(encode_plan(target(), &manual).cap_mbps, 50);
    }

    #[test]
    fn always_4_2_0_so_the_app_player_can_play_it() {
        // 4:4:4 (`-rgb_mode yuv444`) was removed; it must never come back
        // by accident, for any codec.
        let dir = PathBuf::from("buf");
        for codec in Codec::ALL {
            let v = VideoConfig { codec, ..Default::default() };
            let s = record_args(&RecordSpec {
                target: target(),
                video: &v,
                audio: &[],
                mixed_track: true,
                audio_bitrate_kbps: 160,
                segment_dir: &dir,
                start_index: 0,
                audio_origin_unix_us: 0,
            })
            .join(" ");
            assert!(!s.contains("rgb_mode"), "{codec:?}");
        }
    }

    #[test]
    fn no_mix_without_two_sources_or_when_disabled() {
        let v = VideoConfig::default();
        let dir = PathBuf::from("buf");
        let both = game_and_mic();
        let one = &both[..1];
        for (audio, mixed_track) in [(&both[..], false), (one, true)] {
            let spec = RecordSpec {
                target: target(),
                video: &v,
                audio,
                mixed_track,
                audio_bitrate_kbps: 160,
                segment_dir: &dir,
                start_index: 0,
                audio_origin_unix_us: 0,
            };
            let s = record_args(&spec).join(" ");
            assert!(!s.contains("amix"));
            assert!(!s.contains("[mix]"));
            assert_eq!(track_titles(&spec).len(), audio.len());
        }
    }

    #[test]
    fn av1_segments_in_matroska() {
        let dir = PathBuf::from("buf");
        let args_for = |codec| {
            let v = VideoConfig { codec, ..Default::default() };
            record_args(&RecordSpec {
                target: target(),
                video: &v,
                audio: &[],
                mixed_track: true,
                audio_bitrate_kbps: 160,
                segment_dir: &dir,
                start_index: 0,
                audio_origin_unix_us: 0,
            })
            .join(" ")
        };
        let av1 = args_for(Codec::Av1);
        assert!(av1.contains("-segment_format matroska -segment_start_number 0 -reset_timestamps 1"));
        assert!(av1.ends_with("seg_%06d.mkv"));
        let h264 = args_for(Codec::H264);
        assert!(h264.contains("-segment_format mpegts -segment_start_number 0 -reset_timestamps 0"));
        assert!(h264.ends_with("seg_%06d.ts"));
    }

    #[test]
    fn video_only_has_no_audio_codec() {
        let dir = PathBuf::from("buf");
        let v = VideoConfig::default();
        let s = record_args(&RecordSpec {
            target: target(),
            video: &v,
            audio: &[],
            mixed_track: true,
            audio_bitrate_kbps: 160,
            segment_dir: &dir,
            start_index: 0,
            audio_origin_unix_us: 0,
        })
        .join(" ");
        assert!(!s.contains("-c:a"));
        assert!(!s.contains("0:a"));
        // No audio to line up with, so capture times are kept as they are.
        assert!(!s.contains("setpts"));
        assert!(s.contains("capture_cursor=1,select="));
    }
}
