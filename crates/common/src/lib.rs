//! Windows plumbing shared by the tray app (`playerstv.exe`) and the UI
//! (`playerstv-ui.exe`): file locations, the tray's control pipe and status
//! type, and child-process helpers.

pub mod control;
pub mod paths;
pub mod process;
pub mod status;
