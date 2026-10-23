//! Turning a run of buffer segments into a single .mp4 via stream copy.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Local};

use crate::config::{Codec, Quality};
use crate::profile::safe_name;
use crate::ring::Segment;

/// Kind of export, which decides the output sub-folder and file name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportKind {
    Clip,
    Session,
}

/// `<output>/<Game>/<Game>_2026-10-21_14-03-22.mp4` for clips,
/// `<output>/<Game>/Sessions/<Game>_session_<time>.mp4` for full sessions.
pub fn output_path(output_dir: &Path, game: &str, kind: ExportKind, at: DateTime<Local>) -> PathBuf {
    let game = safe_name(game);
    let stamp = at.format("%Y-%m-%d_%H-%M-%S");
    let dir = output_dir.join(&game);
    match kind {
        ExportKind::Clip => dir.join(format!("{game}_{stamp}.mp4")),
        ExportKind::Session => dir.join("Sessions").join(format!("{game}_session_{stamp}.mp4")),
    }
}

/// Returns `path` or, if it already exists, `path` with ` (2)`, ` (3)`… added.
pub fn unique_path(path: PathBuf) -> PathBuf {
    unique_path_except(path, None)
}

/// Like [`unique_path`], but `except` (a file being renamed) doesn't count
/// as taken.
pub fn unique_path_except(path: PathBuf, except: Option<&Path>) -> PathBuf {
    // Windows file names are case-insensitive.
    let free = |p: &Path| !p.exists() || except.is_some_and(|e| e.as_os_str().eq_ignore_ascii_case(p.as_os_str()));
    if free(&path) {
        return path;
    }
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("clip").to_owned();
    let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("mp4").to_owned();
    (2..).map(|n| path.with_file_name(format!("{stem} ({n}).{ext}"))).find(|p| free(p)).expect("unbounded range")
}

/// Longest name a user can give a recording (the timestamp comes on top).
pub const MAX_NAME_CHARS: usize = 100;

/// A name the user typed for a recording, trimmed and without a typed
/// `.mp4`, or why it can't be a file name. Mirrored in ui/src/lib/library.ts.
pub fn check_name(name: &str) -> Result<&str, String> {
    let mut name = name.trim();
    if name.len() >= 4 && name.get(name.len() - 4..).is_some_and(|ext| ext.eq_ignore_ascii_case(".mp4")) {
        name = name[..name.len() - 4].trim_end();
    }
    if name.is_empty() {
        return Err("Enter a name".into());
    }
    if name.chars().count() > MAX_NAME_CHARS {
        return Err(format!("Names can be at most {MAX_NAME_CHARS} characters"));
    }
    if let Some(c) = name.chars().find(|c| r#"<>:"/\|?*"#.contains(*c) || c.is_control()) {
        return Err(if c.is_control() {
            "Names can't contain control characters".into()
        } else {
            format!("Names can't contain {c}")
        });
    }
    Ok(name)
}

/// Where the recording at (or to be saved at) `path` goes when named
/// `name`: `<dir>/<name>_<its timestamp>.mp4`. A name that already ends in a
/// timestamp, like the default one, is used as it is.
pub fn named_path(path: &Path, name: &str) -> Result<PathBuf, String> {
    let name = check_name(name)?;
    let file = format!("{name}.mp4");
    if crate::storage::is_recording_file(&file) {
        return Ok(path.with_file_name(file));
    }
    let (_, stamp) = path
        .file_name()
        .and_then(|n| n.to_str())
        .and_then(crate::storage::split_name)
        .ok_or_else(|| format!("{} isn't a playerstv recording", path.display()))?;
    Ok(path.with_file_name(format!("{name}_{stamp}.mp4")))
}

/// Contents of an ffmpeg concat-demuxer list file.
pub fn concat_list(segments: &[Segment]) -> String {
    let mut out = String::new();
    for seg in segments {
        // Concat list quoting: wrap in '…' and escape embedded quotes as '\''.
        let p = seg.path.to_string_lossy().replace('\\', "/").replace('\'', r"'\''");
        let _ = writeln!(out, "file '{p}'");
    }
    out
}

/// ffmpeg arguments that stream-copy the concat list into `output`.
/// `audio_titles` names the audio tracks in order (e.g. ["Game", "Mic"]).
pub fn export_args(list_file: &Path, output: &Path, audio_titles: &[&str]) -> Vec<String> {
    let mut args: Vec<String> = ["-hide_banner", "-nostdin", "-loglevel", "error", "-y", "-f", "concat", "-safe", "0"]
        .map(String::from)
        .to_vec();
    args.push("-i".into());
    args.push(list_file.to_string_lossy().into_owned());
    args.extend(["-map", "0", "-c", "copy"].map(String::from));
    for (i, title) in audio_titles.iter().enumerate() {
        // MP4 keeps `title` as a "name" atom; most players and editors show
        // the handler name instead, so set both.
        for key in ["title", "handler_name"] {
            args.push(format!("-metadata:s:a:{i}"));
            args.push(format!("{key}={title}"));
        }
    }
    args.extend(["-movflags", "+faststart"].map(String::from));
    args.push(output.to_string_lossy().into_owned());
    args
}

/// A cut from `start` to `end` seconds of `source`, frame-exact: re-encoded
/// on the GPU with `codec` at `quality`, every audio track stream-copied.
/// The output is 4:2:0 even from old 4:4:4 recordings, so it plays in the
/// app. Progress goes to stdout as `-progress` key=value lines.
pub fn trim_args(
    source: &Path,
    start: f64,
    end: f64,
    output: &Path,
    codec: Codec,
    quality: Quality,
    audio_titles: &[&str],
) -> Vec<String> {
    let mut args: Vec<String> = ["-hide_banner", "-nostdin", "-loglevel", "error", "-y"].map(String::from).to_vec();
    // As input options, -ss/-to seek exactly: ffmpeg decodes from the
    // keyframe before `start` and drops frames until it.
    args.extend(["-ss".into(), format!("{start:.6}"), "-to".into(), format!("{end:.6}"), "-i".into()]);
    args.push(source.to_string_lossy().into_owned());
    args.extend(["-map", "0:v:0", "-map", "0:a?", "-map_metadata", "0"].map(String::from));
    let cq = quality.cq(codec).to_string();
    args.extend(
        [
            "-c:v",
            codec.nvenc_encoder(),
            "-preset",
            "p5",
            "-tune",
            "hq",
            "-rc",
            "vbr",
            "-cq",
            &cq,
            "-b:v",
            "0",
            "-spatial-aq",
            "1",
            "-temporal-aq",
            "1",
            "-bf",
            "3",
            "-b_ref_mode",
            "middle",
            "-multipass",
            "qres",
            "-rc-lookahead",
            "16",
            "-pix_fmt",
            "yuv420p",
            "-c:a",
            "copy",
        ]
        .map(String::from),
    );
    for (i, title) in audio_titles.iter().enumerate() {
        for key in ["title", "handler_name"] {
            args.push(format!("-metadata:s:a:{i}"));
            args.push(format!("{key}={title}"));
        }
    }
    // The output name doesn't end in .mp4 until it's complete.
    args.extend(["-movflags", "+faststart", "-f", "mp4", "-progress", "pipe:1", "-nostats"].map(String::from));
    args.push(output.to_string_lossy().into_owned());
    args
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn paths() {
        let t = Local.with_ymd_and_hms(2026, 10, 21, 14, 3, 22).unwrap();
        let out = Path::new("C:/Videos/playerstv");
        assert_eq!(
            output_path(out, "RuneLite", ExportKind::Clip, t),
            out.join("RuneLite").join("RuneLite_2026-10-21_14-03-22.mp4")
        );
        assert_eq!(
            output_path(out, "Rune Lite", ExportKind::Session, t),
            out.join("Rune_Lite").join("Sessions").join("Rune_Lite_session_2026-10-21_14-03-22.mp4")
        );
    }

    #[test]
    fn names_are_checked() {
        assert_eq!(check_name("  Big fight  "), Ok("Big fight"));
        assert_eq!(check_name("Big fight.MP4"), Ok("Big fight"));
        assert!(check_name("   ").is_err());
        assert!(check_name(".mp4").is_err());
        assert_eq!(check_name("a/b"), Err("Names can't contain /".into()));
        assert!(check_name("tab\there").is_err());
        assert!(check_name(&"x".repeat(MAX_NAME_CHARS)).is_ok());
        assert!(check_name(&"x".repeat(MAX_NAME_CHARS + 1)).is_err());
        assert_eq!(check_name("épée"), Ok("épée"));
    }

    #[test]
    fn named_paths_keep_the_timestamp() {
        let dir = Path::new(r"C:\out\RuneLite");
        let clip = dir.join("RuneLite_2026-10-21_14-03-22.mp4");
        assert_eq!(named_path(&clip, "Big fight").unwrap(), dir.join("Big fight_2026-10-21_14-03-22.mp4"));
        // Renaming a named one, or one with a (2), keeps the original timestamp.
        let named = dir.join("Big fight_2026-10-21_14-03-22 (2).mp4");
        assert_eq!(named_path(&named, "Bigger fight").unwrap(), dir.join("Bigger fight_2026-10-21_14-03-22.mp4"));
        // The default name, left alone, stays exactly as it is.
        assert_eq!(named_path(&clip, "RuneLite_2026-10-21_14-03-22").unwrap(), clip);
        let session = dir.join("Sessions").join("RuneLite_session_2026-10-21_14-03-22.mp4");
        assert_eq!(
            named_path(&session, "Raid night").unwrap(),
            dir.join("Sessions").join("Raid night_2026-10-21_14-03-22.mp4")
        );
        assert!(named_path(&clip, "a:b").is_err());
        assert!(named_path(&dir.join("holiday.mp4"), "x").is_err());
    }

    #[test]
    fn renaming_to_its_own_name_is_free() {
        let dir = std::env::temp_dir().join(format!("ptv-clip-unique-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let a = dir.join("A_2026-10-21_14-03-22.mp4");
        let a2 = dir.join("A_2026-10-21_14-03-22 (2).mp4");
        std::fs::write(&a, b"x").unwrap();
        std::fs::write(&a2, b"x").unwrap();
        let picked = (
            unique_path(a.clone()),
            unique_path_except(a.clone(), Some(&a2)),
            unique_path_except(dir.join("a_2026-10-21_14-03-22.mp4"), Some(&a)),
        );
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(picked.0, dir.join("A_2026-10-21_14-03-22 (3).mp4"));
        assert_eq!(picked.1, a2, "the file being renamed can keep its (2)");
        assert_eq!(picked.2, dir.join("a_2026-10-21_14-03-22.mp4"), "a change of case only");
    }

    #[test]
    fn concat_list_escapes() {
        let segs = vec![
            Segment { index: 1, path: PathBuf::from(r"C:\Users\Some User\buf\seg_000001.ts") },
            Segment { index: 2, path: PathBuf::from(r"C:\it's\seg_000002.ts") },
        ];
        assert_eq!(
            concat_list(&segs),
            "file 'C:/Users/Some User/buf/seg_000001.ts'\nfile 'C:/it'\\''s/seg_000002.ts'\n"
        );
    }

    #[test]
    fn export_args_titles_tracks() {
        let a = export_args(Path::new("list.txt"), Path::new("out.mp4"), &["Game", "Mic"]);
        let s = a.join(" ");
        assert!(s.contains("-f concat -safe 0 -i list.txt -map 0 -c copy"));
        assert!(s.contains("-metadata:s:a:0 title=Game -metadata:s:a:0 handler_name=Game"));
        assert!(s.contains("-metadata:s:a:1 title=Mic -metadata:s:a:1 handler_name=Mic"));
        assert!(s.ends_with("-movflags +faststart out.mp4"));
    }

    #[test]
    fn trim_args_cut_exactly_on_the_gpu() {
        let a = trim_args(
            Path::new("in.mp4"),
            10.0,
            15.25,
            Path::new("out.part"),
            Codec::Av1,
            Quality::High,
            &["Mixed", "Mic"],
        );
        let s = a.join(" ");
        assert!(s.contains("-ss 10.000000 -to 15.250000 -i in.mp4 -map 0:v:0 -map 0:a? -map_metadata 0"));
        assert!(s.contains("-c:v av1_nvenc -preset p5 -tune hq -rc vbr -cq 32 -b:v 0"));
        assert!(s.contains("-pix_fmt yuv420p -c:a copy"));
        assert!(s.contains("-metadata:s:a:1 title=Mic -metadata:s:a:1 handler_name=Mic"));
        assert!(s.ends_with("-movflags +faststart -f mp4 -progress pipe:1 -nostats out.part"));
        let h264 = trim_args(Path::new("a"), 0.0, 1.0, Path::new("b"), Codec::H264, Quality::High, &[]).join(" ");
        assert!(h264.contains("-c:v h264_nvenc") && h264.contains("-cq 22") && !h264.contains("-metadata:s:a"));
    }
}
