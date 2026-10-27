//! What the test game writes about its own frames: when each one was
//! presented and how long the game spent on it. The harness reads it to see
//! how much a recorder slows the game down, and the analyzer reads it to
//! tell frames the game skipped from frames a recorder dropped.

use std::io::{BufRead, Write};
use std::path::Path;

use anyhow::{Context, Result};

use crate::pattern::Pattern;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameRecord {
    pub index: u64,
    /// When `Present` returned, in microseconds since the first frame.
    pub present_us: u64,
    /// Drawing, uploading and presenting this frame, in microseconds.
    pub work_us: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FrameLog {
    /// When frame 0 started, in microseconds since the Unix epoch, so other
    /// processes can line their own timings up with the frames.
    pub t0_unix_us: u64,
    /// The game's frame size and rate, which the analyzer needs to redraw
    /// reference frames.
    pub pattern: Pattern,
    pub frames: Vec<FrameRecord>,
}

const META_PREFIX: &str = "# ";
const HEADER: &str = "index,present_us,work_us";

impl FrameLog {
    pub fn write(&self, path: &Path) -> Result<()> {
        let file = std::fs::File::create(path).with_context(|| path.display().to_string())?;
        let mut out = std::io::BufWriter::new(file);
        let p = &self.pattern;
        writeln!(
            out,
            "{META_PREFIX}t0_unix_us={} width={} height={} fps={}",
            self.t0_unix_us, p.width, p.height, p.fps
        )?;
        writeln!(out, "{HEADER}")?;
        for f in &self.frames {
            writeln!(out, "{},{},{}", f.index, f.present_us, f.work_us)?;
        }
        out.flush()?;
        Ok(())
    }

    pub fn read(path: &Path) -> Result<Self> {
        let file = std::fs::File::open(path).with_context(|| path.display().to_string())?;
        let mut meta: Option<(u64, Pattern)> = None;
        let mut frames = Vec::new();
        for (n, line) in std::io::BufReader::new(file).lines().enumerate() {
            let line = line?;
            if let Some(fields) = line.strip_prefix(META_PREFIX) {
                meta = Some(parse_meta(fields).with_context(|| format!("{}: bad header {line:?}", path.display()))?);
                continue;
            }
            if line == HEADER || line.is_empty() {
                continue;
            }
            let parse = || -> Option<FrameRecord> {
                let mut cols = line.split(',').map(|c| c.parse::<u64>().ok());
                Some(FrameRecord { index: cols.next()??, present_us: cols.next()??, work_us: cols.next()?? })
            };
            frames.push(parse().with_context(|| format!("{} line {}: {line:?}", path.display(), n + 1))?);
        }
        let (t0_unix_us, pattern) = meta.with_context(|| format!("{} has no header", path.display()))?;
        Ok(FrameLog { t0_unix_us, pattern, frames })
    }

    /// Milliseconds between consecutive presents within the Unix-time
    /// window `[from_unix_us, to_unix_us]`.
    pub fn intervals_ms(&self, from_unix_us: u64, to_unix_us: u64) -> Vec<f64> {
        let from = from_unix_us.saturating_sub(self.t0_unix_us);
        let to = to_unix_us.saturating_sub(self.t0_unix_us);
        self.frames
            .windows(2)
            .filter(|w| w[0].present_us >= from && w[1].present_us <= to)
            .map(|w| (w[1].present_us - w[0].present_us) as f64 / 1000.0)
            .collect()
    }
}

fn parse_meta(fields: &str) -> Result<(u64, Pattern)> {
    let mut t0 = None;
    let (mut w, mut h, mut fps) = (None, None, None);
    for field in fields.split_whitespace() {
        let (key, value) = field.split_once('=').context("expected key=value")?;
        let value: u64 = value.parse()?;
        match key {
            "t0_unix_us" => t0 = Some(value),
            "width" => w = Some(value as u32),
            "height" => h = Some(value as u32),
            "fps" => fps = Some(value as u32),
            _ => {}
        }
    }
    Ok((t0.context("t0_unix_us")?, Pattern::new(w.context("width")?, h.context("height")?, fps.context("fps")?)))
}

pub fn unix_us_now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_micros() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_windows() {
        let log = FrameLog {
            t0_unix_us: 1_000_000,
            pattern: Pattern::new(1920, 1080, 60),
            frames: (0..5).map(|i| FrameRecord { index: i, present_us: i * 16_667, work_us: 900 }).collect(),
        };
        let path = std::env::temp_dir().join(format!("ptv-frame-log-{}.csv", std::process::id()));
        log.write(&path).unwrap();
        assert_eq!(FrameLog::read(&path).unwrap(), log);
        std::fs::remove_file(&path).unwrap();

        let ms = log.intervals_ms(1_000_000 + 16_667, 1_000_000 + 4 * 16_667);
        assert_eq!(ms.len(), 3);
        assert!(ms.iter().all(|&m| (m - 16.667).abs() < 1e-9));
    }
}
