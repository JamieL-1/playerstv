//! Reading what's inside a recording (duration, video codec and size, audio
//! track names) from its MP4 `moov` box.
//!
//! A tiny purpose-built reader: it seeks past `mdat` (sessions can be many
//! GB) and reads only the boxes the library needs, including the HEVC and
//! AV1 configs and the track names (`hdlr`) that general MP4 crates skip.

use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MediaInfo {
    pub duration_s: f64,
    pub video: Option<VideoInfo>,
    /// Audio track names in file order, e.g. ["Mixed", "Game", "Mic"].
    pub audio_tracks: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VideoInfo {
    /// "h264", "hevc", "av1", or the sample entry's fourcc for anything else.
    pub codec: String,
    pub width: u32,
    pub height: u32,
    /// Average frames per second (samples / track duration).
    pub fps: f64,
    /// 4:2:0 H.264/HEVC/AV1, which the app's player (WebView2) can decode.
    /// False for 4:4:4 / 4:2:2 profiles (older playerstv versions recorded
    /// those) and unknown codecs.
    pub playable: bool,
}

/// Larger `moov` boxes than this aren't ours (hours of 120 fps video need a
/// few MB of sample tables).
const MAX_MOOV: u64 = 256 * 1024 * 1024;

pub fn probe(path: &Path) -> Result<MediaInfo> {
    let mut f = std::fs::File::open(path).with_context(|| format!("open {}", path.display()))?;
    probe_reader(&mut f)
}

pub fn probe_reader<R: Read + Seek>(r: &mut R) -> Result<MediaInfo> {
    let moov = find_moov(r)?;
    parse_moov(&moov)
}

/// Walks the top-level boxes, seeking over everything but `moov`.
fn find_moov<R: Read + Seek>(r: &mut R) -> Result<Vec<u8>> {
    let mut pos = 0u64;
    loop {
        let mut head = [0u8; 8];
        if r.read_exact(&mut head).is_err() {
            bail!("no moov box (unfinished or not an MP4)");
        }
        let mut size = u32::from_be_bytes(head[..4].try_into().expect("4 of 8 bytes")) as u64;
        let mut header_len = 8;
        if size == 1 {
            let mut large = [0u8; 8];
            r.read_exact(&mut large)?;
            size = u64::from_be_bytes(large);
            header_len = 16;
        } else if size == 0 {
            // Box runs to the end of the file.
            size = r.seek(SeekFrom::End(0))? - pos;
            r.seek(SeekFrom::Start(pos + header_len))?;
        }
        if size < header_len {
            bail!("corrupt box at {pos}");
        }
        if &head[4..] == b"moov" {
            let len = size - header_len;
            if len > MAX_MOOV {
                bail!("moov box too large ({len} bytes)");
            }
            let mut buf = vec![0u8; len as usize];
            r.read_exact(&mut buf).context("truncated moov box")?;
            return Ok(buf);
        }
        pos += size;
        r.seek(SeekFrom::Start(pos))?;
    }
}

/// Child boxes of a box's payload: (type, payload).
fn boxes(mut data: &[u8]) -> impl Iterator<Item = (&[u8; 4], &[u8])> {
    std::iter::from_fn(move || {
        if data.len() < 8 {
            return None;
        }
        let size32 = u32::from_be_bytes(data[..4].try_into().expect("len >= 8")) as usize;
        let kind: &[u8; 4] = data[4..8].try_into().expect("len >= 8");
        let (size, header) = match size32 {
            0 => (data.len(), 8),
            1 if data.len() >= 16 => (u64::from_be_bytes(data[8..16].try_into().expect("len >= 16")) as usize, 16),
            _ => (size32, 8),
        };
        if size < header || size > data.len() {
            return None;
        }
        let payload = &data[header..size];
        data = &data[size..];
        Some((kind, payload))
    })
}

fn child<'data>(data: &'data [u8], kind: &[u8; 4]) -> Option<&'data [u8]> {
    boxes(data).find(|(k, _)| *k == kind).map(|(_, p)| p)
}

fn be_u16(d: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes(d.get(at..at + 2)?.try_into().ok()?))
}

fn be_u32(d: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(d.get(at..at + 4)?.try_into().ok()?))
}

fn be_u64(d: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_be_bytes(d.get(at..at + 8)?.try_into().ok()?))
}

/// (timescale, duration) from an `mvhd` or `mdhd` payload; both share the
/// same layout up to the duration.
fn timescale_and_duration(d: &[u8]) -> Option<(u32, u64)> {
    match d.first()? {
        0 => Some((be_u32(d, 12)?, be_u32(d, 16)? as u64)),
        1 => Some((be_u32(d, 20)?, be_u64(d, 24)?)),
        _ => None,
    }
}

fn seconds((timescale, duration): (u32, u64)) -> Option<f64> {
    (timescale > 0 && duration != u32::MAX as u64 && duration != u64::MAX).then(|| duration as f64 / timescale as f64)
}

struct Track<'data> {
    handler: [u8; 4],
    name: String,
    seconds: Option<f64>,
    sample_entry: Option<(&'data [u8; 4], &'data [u8])>,
    samples: u64,
}

fn parse_trak(trak: &[u8]) -> Option<Track<'_>> {
    let mdia = child(trak, b"mdia")?;
    let hdlr = child(mdia, b"hdlr")?;
    let handler: [u8; 4] = hdlr.get(8..12)?.try_into().ok()?;
    let seconds = child(mdia, b"mdhd").and_then(timescale_and_duration).and_then(seconds);
    let stbl = child(child(mdia, b"minf")?, b"stbl")?;
    let sample_entry = child(stbl, b"stsd").and_then(|stsd| boxes(stsd.get(8..)?).next());
    let samples = child(stbl, b"stts").map(stts_samples).unwrap_or(0);
    Some(Track { handler, name: handler_name(hdlr.get(24..).unwrap_or_default()), seconds, sample_entry, samples })
}

/// `hdlr` names are C strings (ISO) or, from some muxers, Pascal strings.
fn handler_name(raw: &[u8]) -> String {
    let raw = match raw.split_first() {
        Some((&len, rest)) if len as usize == rest.len() && len > 0 && rest.first() != Some(&0) && len < 0x20 => rest,
        _ => raw,
    };
    let end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
    String::from_utf8_lossy(&raw[..end]).trim().to_owned()
}

fn stts_samples(stts: &[u8]) -> u64 {
    let count = be_u32(stts, 4).unwrap_or(0) as usize;
    (0..count).filter_map(|i| be_u32(stts, 8 + i * 8)).map(u64::from).sum()
}

fn parse_moov(moov: &[u8]) -> Result<MediaInfo> {
    let movie_seconds = child(moov, b"mvhd").and_then(timescale_and_duration).and_then(seconds);
    let tracks: Vec<Track> = boxes(moov).filter(|(k, _)| *k == b"trak").filter_map(|(_, t)| parse_trak(t)).collect();
    if tracks.is_empty() {
        bail!("no tracks");
    }

    let video = tracks.iter().find(|t| &t.handler == b"vide").map(video_info);
    let audio_tracks = tracks
        .iter()
        .filter(|t| &t.handler == b"soun")
        .enumerate()
        .map(|(i, t)| {
            // ffmpeg's default names say nothing useful.
            if t.name.is_empty() || t.name == "SoundHandler" {
                format!("Track {}", i + 1)
            } else {
                t.name.clone()
            }
        })
        .collect();
    let longest_track =
        tracks.iter().filter_map(|t| t.seconds).fold(None, |m: Option<f64>, s| Some(m.map_or(s, |m| m.max(s))));
    Ok(MediaInfo { duration_s: movie_seconds.or(longest_track).unwrap_or(0.0), video, audio_tracks })
}

fn video_info(t: &Track) -> VideoInfo {
    let fps = match t.seconds {
        Some(s) if s > 0.0 && t.samples > 0 => t.samples as f64 / s,
        _ => 0.0,
    };
    let Some((fourcc, entry)) = t.sample_entry else {
        return VideoInfo { codec: "unknown".into(), width: 0, height: 0, fps, playable: false };
    };
    let width = be_u16(entry, 24).unwrap_or(0) as u32;
    let height = be_u16(entry, 26).unwrap_or(0) as u32;
    // Codec config boxes follow the 78-byte visual sample entry.
    let config = |kind: &[u8; 4]| entry.get(78..).and_then(|d| child(d, kind));
    let (codec, playable) = match fourcc {
        b"avc1" | b"avc3" => ("h264".to_owned(), config(b"avcC").is_some_and(avc_playable)),
        b"hvc1" | b"hev1" => ("hevc".to_owned(), config(b"hvcC").is_some_and(hevc_playable)),
        b"av01" => ("av1".to_owned(), config(b"av1C").is_some_and(av1_playable)),
        other => (String::from_utf8_lossy(other).trim().to_owned(), false),
    };
    VideoInfo { codec, width, height, fps, playable }
}

/// avcC byte 1 is profile_idc. 4:2:2 and 4:4:4 profiles (High 4:2:2 = 122,
/// High 4:4:4 Predictive = 244, CAVLC 4:4:4 Intra = 44) won't decode.
fn avc_playable(avcc: &[u8]) -> bool {
    avcc.get(1).is_some_and(|p| !matches!(p, 122 | 244 | 44))
}

/// hvcC byte 16's low two bits are chroma_format_idc; 1 = 4:2:0.
fn hevc_playable(hvcc: &[u8]) -> bool {
    hvcc.get(16).is_some_and(|b| b & 0b11 == 1)
}

/// av1C byte 1's top three bits are seq_profile; 0 (Main) is 4:2:0 / mono.
fn av1_playable(av1c: &[u8]) -> bool {
    av1c.get(1).is_some_and(|b| b >> 5 == 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn fixture(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests").join("fixtures").join(name)
    }

    /// The fixtures are 0.5 s of 64x36 at 30 fps with "Mixed" and "Mic"
    /// audio tracks (see tests/fixtures/README.md).
    fn check(name: &str, codec: &str, playable: bool) {
        let info = probe(&fixture(name)).unwrap();
        let v = info.video.as_ref().unwrap();
        assert_eq!((v.codec.as_str(), v.width, v.height, v.playable), (codec, 64, 36, playable), "{name}");
        assert!((v.fps - 30.0).abs() < 0.5, "{name}: fps {}", v.fps);
        assert!((info.duration_s - 0.5).abs() < 0.1, "{name}: duration {}", info.duration_s);
    }

    #[test]
    fn reads_each_codec() {
        check("h264.mp4", "h264", true);
        check("hevc.mp4", "hevc", true);
        check("av1.mp4", "av1", true);
    }

    #[test]
    fn flags_444_as_unplayable() {
        check("h264_444_moov_at_end.mp4", "h264", false);
        let info = probe(&fixture("hevc_444.mp4")).unwrap();
        assert!(!info.video.unwrap().playable);
    }

    #[test]
    fn names_audio_tracks() {
        assert_eq!(probe(&fixture("h264.mp4")).unwrap().audio_tracks, ["Mixed", "Mic"]);
        assert!(probe(&fixture("hevc_444.mp4")).unwrap().audio_tracks.is_empty());
    }

    #[test]
    fn rejects_files_without_moov() {
        let mut half = std::io::Cursor::new(b"\0\0\0\x10ftypisom\0\0\0\0\0\0\0\x10mdat\0\0\0\0\0\0\0\0".to_vec());
        assert!(probe_reader(&mut half).unwrap_err().to_string().contains("no moov"));
        let mut garbage = std::io::Cursor::new(b"\0\0\0\x02junk".to_vec());
        assert!(probe_reader(&mut garbage).is_err());
    }

    #[test]
    fn handler_names() {
        assert_eq!(handler_name(b"Mixed\0"), "Mixed");
        assert_eq!(handler_name(b"\x03Mic"), "Mic");
        assert_eq!(handler_name(b""), "");
    }
}
