//! Platform-independent logic for playerstv: configuration, game profiles,
//! ffmpeg command construction and replay-buffer segment bookkeeping.

pub mod clip;
pub mod config;
pub mod ffmpeg_args;
pub mod profile;
pub mod ring;
pub mod storage;
