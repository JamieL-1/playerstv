//! The replay buffer is a directory of short MPEG-TS segments written by
//! ffmpeg's segment muxer. These helpers decide which segments make up a clip
//! and which ones are old enough to delete.

use std::path::{Path, PathBuf};

use crate::config::Codec;

/// Length of each segment in seconds. Clips are accurate to about this much.
pub const SEGMENT_SECONDS: u32 = 2;

/// Extra seconds kept beyond the buffer length so an in-flight clip export
/// never has a segment deleted from under it.
pub const PRUNE_MARGIN_SECONDS: u32 = 10;

/// Segment file format. MPEG-TS survives being cut off mid-write (crashes,
/// the in-progress segment), but FFmpeg reads AV1 back out of TS as an
/// unknown data stream, so AV1 uses Matroska instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Container {
    MpegTs,
    Matroska,
}

impl Container {
    pub fn for_codec(codec: Codec) -> Self {
        match codec {
            Codec::H264 | Codec::Hevc => Container::MpegTs,
            Codec::Av1 => Container::Matroska,
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Container::MpegTs => "ts",
            Container::Matroska => "mkv",
        }
    }

    /// Name for ffmpeg's `-segment_format`.
    pub fn muxer(self) -> &'static str {
        match self {
            Container::MpegTs => "mpegts",
            Container::Matroska => "matroska",
        }
    }

    /// Matroska segments must restart at zero or the concat demuxer leaves
    /// gaps between them; TS is fine with continuous timestamps.
    pub fn reset_timestamps(self) -> bool {
        self == Container::Matroska
    }

    /// ffmpeg output pattern, e.g. `seg_%06d.ts`.
    pub fn pattern(self) -> String {
        format!("seg_%06d.{}", self.extension())
    }
}

const EXTENSIONS: [&str; 2] = ["ts", "mkv"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    pub index: u64,
    pub path: PathBuf,
}

pub fn segment_file_name(index: u64, container: Container) -> String {
    format!("seg_{index:06}.{}", container.extension())
}

pub fn parse_index(file_name: &str) -> Option<u64> {
    let (stem, ext) = file_name.strip_prefix("seg_")?.rsplit_once('.')?;
    if !EXTENSIONS.contains(&ext) {
        return None;
    }
    stem.parse().ok()
}

/// Whether segment `index` exists in either container format.
pub fn segment_exists(dir: &Path, index: u64) -> bool {
    [Container::MpegTs, Container::Matroska].iter().any(|c| dir.join(segment_file_name(index, *c)).exists())
}

/// All segments in `dir`, oldest first.
pub fn scan(dir: &Path) -> std::io::Result<Vec<Segment>> {
    let mut segs: Vec<Segment> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name();
            parse_index(name.to_str()?).map(|index| Segment { index, path: e.path() })
        })
        .collect();
    segs.sort_by_key(|s| s.index);
    Ok(segs)
}

/// Number of segments needed to cover `seconds`, plus one because the newest
/// segment is still being written and may be nearly empty.
pub fn segments_for(seconds: u32) -> usize {
    (seconds.div_ceil(SEGMENT_SECONDS) + 1) as usize
}

/// The newest segments covering at least `seconds` of footage.
pub fn select_for_clip(segs: &[Segment], seconds: u32) -> &[Segment] {
    let n = segments_for(seconds).min(segs.len());
    &segs[segs.len() - n..]
}

/// Segments that can be deleted. Anything at or after `keep_from` (the first
/// segment of an active full-session recording) is always kept.
pub fn prunable(segs: &[Segment], buffer_seconds: u32, keep_from: Option<u64>) -> &[Segment] {
    let keep = segments_for(buffer_seconds + PRUNE_MARGIN_SECONDS);
    let mut cut = segs.len().saturating_sub(keep);
    if let Some(first_kept) = keep_from {
        cut = cut.min(segs.partition_point(|s| s.index < first_kept));
    }
    &segs[..cut]
}

/// Index ffmpeg should start numbering from so a restarted encoder never
/// overwrites earlier segments.
pub fn next_index(segs: &[Segment]) -> u64 {
    segs.last().map_or(0, |s| s.index + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn segs(range: std::ops::Range<u64>) -> Vec<Segment> {
        range.map(|i| Segment { index: i, path: PathBuf::from(segment_file_name(i, Container::MpegTs)) }).collect()
    }

    #[test]
    fn names_roundtrip() {
        assert_eq!(segment_file_name(42, Container::MpegTs), "seg_000042.ts");
        assert_eq!(segment_file_name(42, Container::Matroska), "seg_000042.mkv");
        assert_eq!(parse_index("seg_000042.mkv"), Some(42));
        assert_eq!(parse_index("seg_000042.mp4"), None);
        assert_eq!(parse_index("seg_000042.ts"), Some(42));
        assert_eq!(parse_index("seg_1234567.ts"), Some(1_234_567));
        assert_eq!(parse_index("clip.mp4"), None);
        assert_eq!(parse_index("seg_abc.ts"), None);
    }

    #[test]
    fn clip_selection() {
        let all = segs(0..100);
        // 30s / 2s = 15 complete + 1 in-progress.
        let sel = select_for_clip(&all, 30);
        assert_eq!(sel.len(), 16);
        assert_eq!(sel.first().unwrap().index, 84);
        assert_eq!(sel.last().unwrap().index, 99);
        // Odd lengths round up.
        assert_eq!(select_for_clip(&all, 5).len(), 4);
        // Not enough footage yet: take everything.
        assert_eq!(select_for_clip(&segs(0..3), 30).len(), 3);
        assert!(select_for_clip(&[], 30).is_empty());
    }

    #[test]
    fn pruning_respects_buffer_and_margin() {
        let all = segs(0..100);
        let p = prunable(&all, 30, None);
        // keep (30+10)/2 + 1 = 21 segments
        assert_eq!(p.len(), 79);
        assert_eq!(p.last().unwrap().index, 78);
        assert!(prunable(&segs(0..10), 30, None).is_empty());
    }

    #[test]
    fn pruning_keeps_session() {
        let all = segs(0..100);
        assert_eq!(prunable(&all, 30, Some(50)).len(), 50);
        assert!(prunable(&all, 30, Some(0)).is_empty());
        // Session started before anything on disk: keep all.
        assert!(prunable(&segs(10..100), 30, Some(5)).is_empty());
        // Session starts inside the kept window: normal pruning applies.
        assert_eq!(prunable(&all, 30, Some(95)).len(), 79);
    }

    #[test]
    fn next_index_continues() {
        assert_eq!(next_index(&[]), 0);
        assert_eq!(next_index(&segs(3..8)), 8);
    }
}
