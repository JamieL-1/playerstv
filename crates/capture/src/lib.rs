//! Windows-specific capture plumbing: finding the game window and feeding
//! audio into ffmpeg over named pipes. Video capture itself is done inside
//! ffmpeg (gfxcapture), so nothing here touches frames.

pub mod audio;
pub mod detect;
mod pipe;
