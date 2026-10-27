//! Checks a saved clip of the test game: which frames it holds, how close
//! they are to the frames the game drew, and whether sound and picture line
//! up. A recorder's cost only counts if its clip passes these checks.

use std::collections::BTreeSet;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::frame_log::FrameRecord;
use crate::pattern::{self, Pattern, BLOCKS};
use crate::stats;

/// What the clip holds and how good it is.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClipReport {
    pub path: PathBuf,
    pub container: String,
    pub video_codec: String,
    pub width: u32,
    pub height: u32,
    pub audio_tracks: usize,
    pub duration_s: f64,
    pub size_bytes: u64,
    pub bitrate_mbps: f64,
    pub frames: usize,
    /// Frames whose index strip couldn't be read.
    pub unreadable: usize,
    pub first_index: Option<u64>,
    pub last_index: Option<u64>,
    /// Frames the game presented, inside the clip's range, that the clip lacks.
    pub dropped: u64,
    /// Frames that repeat the previous one.
    pub duplicates: u64,
    /// Indices the game skipped itself (not the recorder's fault).
    pub skipped_by_game: u64,
    /// Longest run of consecutive missing frames.
    pub longest_gap: u64,
    pub ssim: Option<f64>,
    pub psnr_db: Option<f64>,
    /// Median of audio click time minus flash frame time; positive means
    /// the sound is late.
    pub av_offset_ms: Option<f64>,
}

/// Where ffprobe is: next to ffmpeg.
pub fn ffprobe_for(ffmpeg: &Path) -> PathBuf {
    ffmpeg.with_file_name("ffprobe.exe")
}

/// `log` is the game's frame log, so frames the game never drew aren't
/// blamed on the recorder; without it every index is assumed drawn.
pub fn analyze(ffmpeg: &Path, clip: &Path, pattern: Pattern, log: Option<&[FrameRecord]>) -> Result<ClipReport> {
    let probe = probe(&ffprobe_for(ffmpeg), clip)?;
    let frames = read_indices(ffmpeg, clip)?;
    let indices: Vec<Option<u64>> = frames.iter().map(|f| f.index).collect();
    let continuity = continuity(&indices, log);

    let (ssim, psnr_db) = match quality(ffmpeg, clip, pattern, &indices) {
        Ok(q) => q,
        Err(e) => {
            eprintln!("quality comparison failed: {e:#}");
            (None, None)
        }
    };
    let av_offset_ms = if probe.audio_tracks > 0 {
        let clicks = click_times(ffmpeg, clip, probe.audio_start_s)?;
        let flashes: Vec<(f64, u64)> = frames
            .iter()
            .filter_map(|f| f.index.filter(|&i| pattern.is_flash(i)).map(|i| (f.pts_s, pattern.click_code(i))))
            .collect();
        av_offset_ms(&flashes, &clicks)
    } else {
        None
    };

    Ok(ClipReport {
        path: clip.to_owned(),
        bitrate_mbps: probe.size_bytes as f64 * 8.0 / probe.duration_s.max(1e-9) / 1e6,
        container: probe.container,
        video_codec: probe.video_codec,
        width: probe.width,
        height: probe.height,
        audio_tracks: probe.audio_tracks,
        duration_s: probe.duration_s,
        size_bytes: probe.size_bytes,
        frames: frames.len(),
        unreadable: indices.iter().filter(|i| i.is_none()).count(),
        first_index: continuity.first,
        last_index: continuity.last,
        dropped: continuity.dropped,
        duplicates: continuity.duplicates,
        skipped_by_game: continuity.skipped_by_game,
        longest_gap: continuity.longest_gap,
        ssim,
        psnr_db,
        av_offset_ms,
    })
}

struct Probe {
    container: String,
    video_codec: String,
    width: u32,
    height: u32,
    audio_tracks: usize,
    audio_start_s: f64,
    duration_s: f64,
    size_bytes: u64,
}

fn probe(ffprobe: &Path, clip: &Path) -> Result<Probe> {
    #[derive(Deserialize)]
    struct Out {
        streams: Vec<Stream>,
        format: Format,
    }
    #[derive(Deserialize)]
    struct Stream {
        codec_type: String,
        codec_name: Option<String>,
        width: Option<u32>,
        height: Option<u32>,
        start_time: Option<String>,
    }
    #[derive(Deserialize)]
    struct Format {
        format_name: String,
        duration: Option<String>,
        size: Option<String>,
    }

    let out = Command::new(ffprobe)
        .args(["-v", "error", "-show_streams", "-show_format", "-of", "json"])
        .arg(clip)
        .output()
        .with_context(|| format!("running {}", ffprobe.display()))?;
    if !out.status.success() {
        bail!("ffprobe failed on {}: {}", clip.display(), String::from_utf8_lossy(&out.stderr));
    }
    let parsed: Out = serde_json::from_slice(&out.stdout).context("parsing ffprobe output")?;
    let num = |s: &Option<String>| s.as_deref().and_then(|s| s.parse::<f64>().ok());
    let video = parsed.streams.iter().find(|s| s.codec_type == "video").context("clip has no video")?;
    let audio: Vec<&Stream> = parsed.streams.iter().filter(|s| s.codec_type == "audio").collect();
    Ok(Probe {
        container: parsed.format.format_name.clone(),
        video_codec: video.codec_name.clone().unwrap_or_default(),
        width: video.width.unwrap_or(0),
        height: video.height.unwrap_or(0),
        audio_tracks: audio.len(),
        audio_start_s: audio.first().and_then(|a| num(&a.start_time)).unwrap_or(0.0),
        duration_s: num(&parsed.format.duration).unwrap_or(0.0),
        size_bytes: num(&parsed.format.size).unwrap_or(0.0) as u64,
    })
}

struct ClipFrame {
    pts_s: f64,
    index: Option<u64>,
}

/// Decodes the index strip of every frame. ffmpeg crops it and scales it
/// to one gray pixel per block, so only 32 bytes a frame come back.
fn read_indices(ffmpeg: &Path, clip: &Path) -> Result<Vec<ClipFrame>> {
    let filter = format!("crop=iw:ih/48:0:0,scale={BLOCKS}:1:flags=area,format=gray,showinfo");
    let mut child = Command::new(ffmpeg)
        .args(["-hide_banner", "-loglevel", "info", "-copyts", "-i"])
        .arg(clip)
        .args(["-map", "0:v:0", "-vf", &filter, "-fps_mode", "passthrough", "-f", "rawvideo", "-"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("running {}", ffmpeg.display()))?;

    let stderr = child.stderr.take().context("ffmpeg stderr")?;
    let pts_reader = std::thread::spawn(move || -> Vec<f64> {
        BufReader::new(stderr)
            .lines()
            .map_while(Result::ok)
            .filter(|l| l.contains("Parsed_showinfo"))
            .filter_map(|l| l.split("pts_time:").nth(1)?.split_whitespace().next()?.parse().ok())
            .collect()
    });

    let mut stdout = child.stdout.take().context("ffmpeg stdout")?;
    let mut indices = Vec::new();
    let mut blocks = [0u8; BLOCKS];
    while read_full(&mut stdout, &mut blocks)? {
        indices.push(pattern::decode(&blocks));
    }
    let status = child.wait()?;
    let pts = pts_reader.join().unwrap_or_default();
    if !status.success() {
        bail!("ffmpeg couldn't decode {}", clip.display());
    }
    if pts.len() != indices.len() {
        bail!("got {} frames but {} timestamps from {}", indices.len(), pts.len(), clip.display());
    }
    Ok(pts.into_iter().zip(indices).map(|(pts_s, index)| ClipFrame { pts_s, index }).collect())
}

/// Fills `buf`; `false` on a clean end of stream.
fn read_full(r: &mut impl Read, buf: &mut [u8]) -> Result<bool> {
    let mut got = 0;
    while got < buf.len() {
        match r.read(&mut buf[got..])? {
            0 if got == 0 => return Ok(false),
            0 => bail!("stream ended mid-frame"),
            n => got += n,
        }
    }
    Ok(true)
}

#[derive(Debug, Default, PartialEq, Eq)]
struct Continuity {
    first: Option<u64>,
    last: Option<u64>,
    dropped: u64,
    duplicates: u64,
    skipped_by_game: u64,
    longest_gap: u64,
}

fn continuity(indices: &[Option<u64>], log: Option<&[FrameRecord]>) -> Continuity {
    let read: Vec<u64> = indices.iter().flatten().copied().collect();
    let (Some(&first), Some(&last)) = (read.iter().min(), read.iter().max()) else {
        return Continuity::default();
    };
    let seen: BTreeSet<u64> = read.iter().copied().collect();
    let drawn: Option<BTreeSet<u64>> =
        log.map(|l| l.iter().map(|f| f.index).filter(|i| (first..=last).contains(i)).collect());
    let was_drawn = |i: u64| drawn.as_ref().is_none_or(|d| d.contains(&i));

    let mut c = Continuity { first: Some(first), last: Some(last), ..Default::default() };
    let mut gap = 0;
    for i in first..=last {
        if seen.contains(&i) {
            gap = 0;
        } else if was_drawn(i) {
            c.dropped += 1;
            gap += 1;
            c.longest_gap = c.longest_gap.max(gap);
        } else {
            c.skipped_by_game += 1;
        }
    }
    c.duplicates = read.windows(2).filter(|w| w[0] == w[1]).count() as u64;
    c
}

/// SSIM and PSNR of the clip against the frames the game drew. Each clip
/// frame is compared with a fresh render of the index it shows, so dropped
/// or repeated frames don't throw the comparison out of step.
fn quality(
    ffmpeg: &Path,
    clip: &Path,
    pattern: Pattern,
    indices: &[Option<u64>],
) -> Result<(Option<f64>, Option<f64>)> {
    let (w, h, fps) = (pattern.width, pattern.height, pattern.fps);
    // Both inputs are renumbered 0, 1, 2… in a 1/fps timebase so the
    // comparison pairs frames by position. `setpts=N/(fps*TB)` would be
    // computed in floating point: with a 1/90000 timebase it can land a
    // tick early, and ssim then pairs each frame with the previous one.
    let renumber = format!("settb=1/{fps},setpts=N");
    let graph = format!(
        "[0:v]{renumber},scale={w}:{h}:flags=bicubic,format=yuv420p,split[a1][a2];\
         [1:v]{renumber},format=yuv420p,split[b1][b2];\
         [a1][b1]ssim[s];[a2][b2]psnr[p]"
    );
    let mut child = Command::new(ffmpeg)
        .args(["-hide_banner", "-nostats", "-loglevel", "info", "-i"])
        .arg(clip)
        .args(["-f", "rawvideo", "-pix_fmt", "bgra", "-s", &format!("{w}x{h}"), "-r", &fps.to_string(), "-i", "-"])
        .args(["-filter_complex", &graph, "-map", "[s]", "-f", "null", "-", "-map", "[p]", "-f", "null", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("running {}", ffmpeg.display()))?;

    let mut stdin = child.stdin.take().context("ffmpeg stdin")?;
    let order: Vec<u64> = indices
        .iter()
        .scan(0, |last, i| {
            *last = i.unwrap_or(*last);
            Some(*last)
        })
        .collect();
    let writer = std::thread::spawn(move || -> std::io::Result<()> {
        let mut frame = vec![0u8; pattern.frame_bytes()];
        for index in order {
            pattern.render(index, &mut frame);
            stdin.write_all(&frame)?;
        }
        Ok(())
    });

    let mut log = String::new();
    child.stderr.take().context("ffmpeg stderr")?.read_to_string(&mut log)?;
    let status = child.wait()?;
    // ffmpeg may stop reading early; a broken pipe there isn't an error.
    let _ = writer.join();
    if !status.success() {
        bail!("ffmpeg quality pass failed: {}", log.lines().last().unwrap_or_default());
    }
    Ok((parse_after(&log, "SSIM", "All:"), parse_after(&log, "PSNR", "average:")))
}

fn parse_after(log: &str, line_has: &str, key: &str) -> Option<f64> {
    let line = log.lines().rev().find(|l| l.contains(line_has) && l.contains(key))?;
    line.split(key).nth(1)?.split_whitespace().next()?.parse().ok()
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Click {
    at_s: f64,
    /// From its length; `None` if that isn't one of the tone's lengths.
    code: Option<u64>,
}

/// The tone's clicks in the first audio track, timed in clip seconds.
fn click_times(ffmpeg: &Path, clip: &Path, audio_start_s: f64) -> Result<Vec<Click>> {
    const RATE: f64 = 48_000.0;
    let out = Command::new(ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-i"])
        .arg(clip)
        .args(["-map", "0:a:0", "-ac", "1", "-ar", "48000", "-f", "f32le", "-"])
        .output()
        .with_context(|| format!("running {}", ffmpeg.display()))?;
    if !out.status.success() {
        bail!("ffmpeg couldn't decode audio: {}", String::from_utf8_lossy(&out.stderr));
    }
    let samples: Vec<f32> = out.stdout.as_chunks::<4>().0.iter().map(|b| f32::from_le_bytes(*b)).collect();
    Ok(find_clicks(&samples, RATE).into_iter().map(|c| Click { at_s: audio_start_s + c.at_s, ..c }).collect())
}

/// Each burst over half the signal's peak, after at least half a second
/// under it, is a click: it starts at its first loud sample and its length
/// gives its code. Bursts cut off by the start or end of the audio are left
/// out, since their start or length would be wrong.
fn find_clicks(samples: &[f32], rate: f64) -> Vec<Click> {
    let peak = samples.iter().fold(0f32, |m, s| m.max(s.abs()));
    if peak == 0.0 {
        return Vec::new();
    }
    let threshold = peak / 2.0;
    let quiet_gap = (rate / 2.0) as usize;
    let edge = (rate / 1000.0) as usize;
    let click = |(first, last): (usize, usize)| -> Option<Click> {
        if first <= edge || last + edge >= samples.len() {
            return None;
        }
        let units = (last - first + 1) as f64 / rate / pattern::CLICK_UNIT_S;
        let code = units.round() - 1.0;
        let fits = (0.0..pattern::CLICK_CODES as f64).contains(&code) && (units - units.round()).abs() < 1.0 / 3.0;
        Some(Click { at_s: first as f64 / rate, code: fits.then_some(code as u64) })
    };

    let mut clicks = Vec::new();
    let mut burst: Option<(usize, usize)> = None;
    for (n, s) in samples.iter().enumerate() {
        if s.abs() < threshold {
            continue;
        }
        match &mut burst {
            Some((_, last)) if n - *last <= quiet_gap => *last = n,
            _ => clicks.extend(burst.replace((n, n)).and_then(click)),
        }
    }
    clicks.extend(burst.and_then(click));
    clicks
}

/// Median offset from each flash to the nearest click with the same code.
/// Clicks with one code are [`pattern::CLICK_CODES`] seconds apart, so the
/// match is unambiguous for sound up to half that early or late.
fn av_offset_ms(flashes: &[(f64, u64)], clicks: &[Click]) -> Option<f64> {
    let limit = pattern::CLICK_CODES as f64 / 2.0;
    let offsets: Vec<f64> = flashes
        .iter()
        .filter_map(|&(v, code)| {
            let a = clicks
                .iter()
                .filter(|c| c.code == Some(code))
                .map(|c| c.at_s)
                .min_by(|a, b| (a - v).abs().total_cmp(&(b - v).abs()))?;
            ((a - v).abs() < limit).then_some((a - v) * 1000.0)
        })
        .collect();
    stats::median(&offsets)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_drops_duplicates_and_game_skips() {
        let idx = |v: &[u64]| v.iter().map(|&i| Some(i)).collect::<Vec<_>>();
        let mut clip = idx(&[10, 11, 11, 14, 15, 17]);
        clip.insert(2, None);
        let c = continuity(&clip, None);
        assert_eq!((c.first, c.last), (Some(10), Some(17)));
        assert_eq!((c.dropped, c.duplicates, c.longest_gap), (3, 1, 2));

        // The game never drew 16, so that one isn't a drop.
        let log: Vec<FrameRecord> =
            (0..30).filter(|&i| i != 16).map(|index| FrameRecord { index, present_us: 0, work_us: 0 }).collect();
        let c = continuity(&clip, Some(&log));
        assert_eq!((c.dropped, c.skipped_by_game), (2, 1));
    }

    /// `seconds` of tone at 1 kHz, each second's click as the game plays
    /// it, with the sound `delay_ms` late.
    fn tone(seconds: u64, delay_ms: usize) -> Vec<f32> {
        let rate = 1000;
        let mut s = vec![0.05f32; seconds as usize * rate + delay_ms];
        for second in 0..seconds {
            let start = second as usize * rate + delay_ms;
            let len = ((pattern::click_code(second) + 1) as f64 * pattern::CLICK_UNIT_S * rate as f64) as usize;
            s[start..start + len].fill(0.8);
        }
        s
    }

    #[test]
    fn finds_clicks_and_offset() {
        let s = tone(6, 100);
        let clicks = find_clicks(&s, 1000.0);
        let codes: Vec<Option<u64>> = clicks.iter().map(|c| c.code).collect();
        assert_eq!(codes, [Some(0), Some(1), Some(2), Some(3), Some(0), Some(1)]);
        assert_eq!(clicks[2].at_s, 2.1);

        let flashes: Vec<(f64, u64)> = (0..6).map(|s| (s as f64 - 0.01 + 0.1, pattern::click_code(s))).collect();
        let off = av_offset_ms(&flashes, &clicks).unwrap();
        assert!((off - 10.0).abs() < 1e-6, "{off}");
    }

    #[test]
    fn sound_a_whole_second_late_is_not_in_sync() {
        let clicks = find_clicks(&tone(8, 1000), 1000.0);
        let flashes: Vec<(f64, u64)> = (0..8).map(|s| (s as f64, pattern::click_code(s))).collect();
        let off = av_offset_ms(&flashes, &clicks).unwrap();
        assert!((off - 1000.0).abs() < 1e-6, "{off}");
    }

    #[test]
    fn parses_ffmpeg_summaries() {
        let log = "[Parsed_ssim_6 @ 0x1] SSIM Y:0.991 (20.4) U:0.99 (20) V:0.99 (20) All:0.990573 (20.27)\n\
                   [Parsed_psnr_7 @ 0x2] PSNR y:41.2 u:44.0 v:44.1 average:42.07 min:39.9 max:45.0";
        assert_eq!(parse_after(log, "SSIM", "All:"), Some(0.990573));
        assert_eq!(parse_after(log, "PSNR", "average:"), Some(42.07));
    }
}
