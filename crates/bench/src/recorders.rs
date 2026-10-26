//! The recorders the benchmark knows: which processes are theirs, how to
//! save a clip, and how to set them up so every one records the same thing.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum Recorder {
    /// Nothing recording: the baseline every other run is compared with.
    None,
    Playerstv,
    Obs,
    Nvidia,
    Medal,
    Outplayed,
    Gamebar,
}

/// What every recorder is asked to record.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub buffer_seconds: u32,
    /// Peak video bitrate to set where the recorder allows it.
    pub bitrate_mbps: u32,
    /// Record the game's audio.
    pub audio: bool,
}

impl Recorder {
    pub const ALL: [Recorder; 7] = [
        Recorder::None,
        Recorder::Playerstv,
        Recorder::Obs,
        Recorder::Nvidia,
        Recorder::Medal,
        Recorder::Outplayed,
        Recorder::Gamebar,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Recorder::None => "No recorder",
            Recorder::Playerstv => "playerstv",
            Recorder::Obs => "OBS replay buffer",
            Recorder::Nvidia => "NVIDIA App Instant Replay",
            Recorder::Medal => "Medal",
            Recorder::Outplayed => "Outplayed",
            Recorder::Gamebar => "Xbox Game Bar",
        }
    }

    /// Executable names of the recorder's processes (`*` matches a prefix).
    /// Their child processes are counted too.
    pub fn processes(self) -> &'static [&'static str] {
        match self {
            Recorder::None => &[],
            Recorder::Playerstv => &["playerstv.exe"],
            Recorder::Obs => &["obs64.exe"],
            Recorder::Nvidia => &["nvidia app.exe", "nvidia overlay.exe", "nvidia share.exe", "nvsphelper64.exe"],
            Recorder::Medal => &["medal*"],
            Recorder::Outplayed => &["overwolf*", "outplayed*"],
            Recorder::Gamebar => &["gamebar*", "broadcastdvrserver.exe", "bcastdvr*"],
        }
    }

    /// The save-clip hotkey the setup checklist asks for.
    pub fn hotkey(self) -> Option<&'static str> {
        match self {
            Recorder::None => None,
            Recorder::Playerstv | Recorder::Nvidia => Some("Alt+F10"),
            Recorder::Obs => Some("Ctrl+Shift+F10"),
            Recorder::Medal => Some("F8"),
            Recorder::Outplayed => Some("Ctrl+Shift+F11"),
            Recorder::Gamebar => Some("Win+Alt+G"),
        }
    }

    /// Where its clips land, when that's fixed. The others take
    /// `--clips-dir`; playerstv's is set by the harness.
    pub fn default_clips_dir(self) -> Option<PathBuf> {
        let videos = std::env::var_os("USERPROFILE").map(|p| PathBuf::from(p).join("Videos"))?;
        match self {
            Recorder::Gamebar => Some(videos.join("Captures")),
            Recorder::Nvidia => Some(videos),
            _ => None,
        }
    }

    /// Whether the harness sets the recorder up itself.
    pub fn automatic(self) -> bool {
        matches!(self, Recorder::None | Recorder::Playerstv)
    }

    /// Steps for setting up a recorder the harness can't configure, so it
    /// records the same thing as the others.
    pub fn checklist(self, s: &Settings) -> Vec<String> {
        let audio = if s.audio { "Game audio on, microphone off." } else { "All audio off, microphone off." };
        let common = [
            format!("Replay buffer length {} s.", s.buffer_seconds),
            format!("{} fps, {}x{} (no downscaling).", s.fps, s.width, s.height),
            audio.to_owned(),
            "Close every other recorder (playerstv, OBS, Medal, Outplayed, NVIDIA Instant Replay, Game Bar)."
                .to_owned(),
        ];
        let specific: Vec<String> = match self {
            Recorder::None | Recorder::Playerstv => return Vec::new(),
            Recorder::Obs => vec![
                "Add a Game Capture or Window Capture source for the window ptv-bench-game.".into(),
                "Output: Advanced. Encoder NVIDIA NVENC H.264, rate control VBR, preset P5.".into(),
                format!("Max bitrate {} Mbps, keyframe interval 2 s.", s.bitrate_mbps),
                "Recording format MP4 (or Fragmented MP4).".into(),
                "Settings, Hotkeys: Save Replay set to Ctrl+Shift+F10. Then click Start Replay Buffer.".into(),
            ],
            Recorder::Nvidia => vec![
                "NVIDIA App, Overlay, Recording: Instant Replay on.".into(),
                format!("Quality Custom: H.264, bitrate {} Mbps.", s.bitrate_mbps),
                "Save hotkey Alt+F10 (the default).".into(),
            ],
            Recorder::Medal => vec![
                "Medal settings, Recorder: H.264 on the NVIDIA encoder, quality closest to the bitrate below.".into(),
                format!("Target bitrate about {} Mbps.", s.bitrate_mbps),
                "Clip hotkey F8 (the default). Add ptv-bench-game.exe as a game if Medal doesn't record it.".into(),
            ],
            Recorder::Outplayed => vec![
                "Outplayed settings: capture with the NVIDIA encoder, quality closest to the bitrate below.".into(),
                format!("Target bitrate about {} Mbps.", s.bitrate_mbps),
                "Manual capture hotkey Ctrl+Shift+F11. Add ptv-bench-game.exe as a game if needed.".into(),
            ],
            Recorder::Gamebar => vec![
                "Windows Settings, Gaming, Captures: Record what happened on, last 30 seconds.".into(),
                format!("Video frame rate {} fps, quality High, capture audio as above.", s.fps),
                "Win+Alt+G saves the clip. Mark ptv-bench-game as a game in the Game Bar (Win+G) if asked.".into(),
            ],
        };
        specific.into_iter().chain(common).collect()
    }
}

impl Recorder {
    /// The process that's only there while the recorder is active. Not
    /// every process in [`Recorder::processes`]: some, like NVIDIA's
    /// container or GameBarPresenceWriter, run whether or not anything
    /// records.
    fn active_process(self) -> Option<&'static str> {
        match self {
            Recorder::None => None,
            Recorder::Playerstv => Some("playerstv.exe"),
            Recorder::Obs => Some("obs64.exe"),
            Recorder::Nvidia => Some("nvidia overlay.exe"),
            Recorder::Medal => Some("medal.exe"),
            Recorder::Outplayed => Some("overwolf.exe"),
            Recorder::Gamebar => Some("broadcastdvrserver.exe"),
        }
    }
}

/// Background recorders the benchmark doesn't compare against but that
/// still encode video while they run.
const OTHER_RECORDERS: [(&str, &str); 1] = [("Discord Clips", "discord_clips.exe")];

/// Recorders other than `under_test` that look active, so a run can warn
/// that they may have skewed its numbers.
pub fn others_running(under_test: Recorder, running: &[String]) -> Vec<&'static str> {
    let is_running = |exe: &str| running.iter().any(|r| r == exe);
    Recorder::ALL
        .into_iter()
        .filter(|&r| r != under_test && r.active_process().is_some_and(is_running))
        .map(Recorder::label)
        .chain(OTHER_RECORDERS.into_iter().filter(|(_, exe)| is_running(exe)).map(|(label, _)| label))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hotkeys_parse() {
        for r in Recorder::ALL {
            if let Some(h) = r.hotkey() {
                crate::input::parse(h).unwrap();
            }
        }
    }

    #[test]
    fn spots_other_recorders() {
        let running = ["explorer.exe".to_owned(), "obs64.exe".to_owned(), "medal.exe".to_owned()];
        assert_eq!(others_running(Recorder::Obs, &running), ["Medal"]);
        // Always-running helpers don't count.
        let idle = ["gamebarpresencewriter.exe".to_owned(), "nvcontainer.exe".to_owned()];
        assert!(others_running(Recorder::None, &idle).is_empty());
    }
}
