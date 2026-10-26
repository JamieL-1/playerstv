//! The Rust work playerstv does around a recording: building the ffmpeg
//! command, scanning the buffer, picking a clip's segments and reading clip
//! metadata. None of it is on the per-frame path (ffmpeg and the GPU are),
//! so these exist to catch a regression that makes saving or the library
//! slow, not to show the recorder's overall cost. That's crates/bench.
use std::path::PathBuf;

use criterion::{criterion_group, criterion_main, Criterion};
use ptv_core::config::{Codec, VideoConfig};
use ptv_core::ffmpeg_args::{record_args, AudioInput, CaptureTarget, RecordSpec};
use ptv_core::ring::{self, Container};
use ptv_core::{clip, media};

fn ffmpeg_command(c: &mut Criterion) {
    let video = VideoConfig { codec: Codec::Hevc, fps: 120, ..Default::default() };
    let audio = [
        AudioInput { pipe_path: r"\\.\pipe\ptv-game".into(), title: "Game".into(), mix_volume: 100 },
        AudioInput { pipe_path: r"\\.\pipe\ptv-mic".into(), title: "Mic".into(), mix_volume: 80 },
    ];
    let dir = PathBuf::from(r"C:\buffer");
    let spec = RecordSpec {
        target: CaptureTarget { hwnd: 0x1234, width: 2560, height: 1440 },
        video: &video,
        audio: &audio,
        mixed_track: true,
        audio_bitrate_kbps: 160,
        segment_dir: &dir,
        start_index: 0,
    };
    c.bench_function("ffmpeg_args::record_args", |b| b.iter(|| record_args(&spec)));
}

/// A buffer folder holding 10 minutes of 2 s segments, the longest buffer.
fn buffer_scan(c: &mut Criterion) {
    let dir = std::env::temp_dir().join(format!("ptv-core-bench-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    for i in 0..300 {
        std::fs::write(dir.join(ring::segment_file_name(i, Container::MpegTs)), b"").expect("segment");
    }
    c.bench_function("ring::scan + select_for_clip (300 segments)", |b| {
        b.iter(|| {
            let segs = ring::scan(&dir).expect("scan");
            ring::select_for_clip(&segs, 30).len()
        })
    });
    let segs = ring::scan(&dir).expect("scan");
    c.bench_function("clip::concat_list (300 segments)", |b| b.iter(|| clip::concat_list(&segs)));
    let _ = std::fs::remove_dir_all(&dir);
}

fn probe(c: &mut Criterion) {
    let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests").join("fixtures");
    for name in ["h264.mp4", "h264_444_moov_at_end.mp4"] {
        let path = fixtures.join(name);
        c.bench_function(&format!("media::probe {name}"), |b| b.iter(|| media::probe(&path).expect("probe")));
    }
}

criterion_group!(benches, ffmpeg_command, buffer_scan, probe);
criterion_main!(benches);
