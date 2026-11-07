//! What the tray is doing, as reported over the control pipe.

use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// Times are Unix seconds so they mean the same thing in the UI's process.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Status {
    Waiting,
    Recording {
        game: String,
        /// When the current encoder started filling the replay buffer.
        since: u64,
        /// When the current full-session file started, if one is recording.
        session_since: Option<u64>,
        /// What's being encoded, e.g. "1900×1030 · 60 fps · H.264 · High quality".
        #[serde(default)]
        video: String,
        /// This game's replay buffer length (games can override the global one).
        #[serde(default)]
        buffer_seconds: u32,
        /// Frames per second actually captured, when that has stayed far
        /// below the target while the game had focus: the game drawing
        /// fewer frames, or capture falling behind.
        #[serde(default)]
        low_fps: Option<u32>,
        /// Whether this game keeps a replay buffer. False while a full
        /// session records a game with the buffer off: no clipping then.
        #[serde(default = "yes")]
        replay_buffer: bool,
    },
    /// Game found, but its replay buffer is off and no full session is
    /// recording, so nothing is being encoded or written.
    Idle {
        game: String,
    },
    /// Game found but the encoder keeps failing.
    Failing {
        game: String,
        error: String,
    },
}

impl Status {
    pub fn session_since(&self) -> Option<u64> {
        match self {
            Status::Recording { session_since, .. } => *session_since,
            _ => None,
        }
    }

    /// The recording game's buffer length, else `default`.
    pub fn buffer_seconds_or(&self, default: u32) -> u32 {
        match self {
            Status::Recording { buffer_seconds, .. } if *buffer_seconds > 0 => *buffer_seconds,
            _ => default,
        }
    }

    pub fn is_recording(&self) -> bool {
        matches!(self, Status::Recording { .. })
    }

    /// A clip can be saved: recording, with a replay buffer.
    pub fn can_clip(&self) -> bool {
        matches!(self, Status::Recording { replay_buffer: true, .. })
    }

    /// A full session can be started or stopped: recording, or a game found
    /// with its replay buffer off.
    pub fn can_toggle_session(&self) -> bool {
        matches!(self, Status::Recording { .. } | Status::Idle { .. })
    }
}

fn yes() -> bool {
    true
}

pub fn unix_now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}
