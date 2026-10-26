//! Windows plumbing shared by the tray and the window (`playerstv.exe` and
//! `playerstv.exe --window`): file locations, the tray's control pipe and status
//! type, child-process helpers and poison-tolerant locking.

pub mod control;
pub mod paths;
pub mod process;
pub mod status;
pub mod sync;
