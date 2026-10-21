//! Keeping saved recordings under a storage limit by deleting the oldest.
//!
//! Only files playerstv itself wrote are ever considered: `.mp4`s directly in
//! `<output>/<Game>/` or `<output>/<Game>/Sessions/` whose names follow our
//! naming scheme (see [`crate::clip::output_path`]). Anything else the user
//! keeps in the output folder is never counted or deleted.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::SystemTime;

use regex::Regex;

pub const BYTES_PER_GB: u64 = 1_000_000_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recording {
    pub path: PathBuf,
    pub size: u64,
    pub modified: SystemTime,
}

/// `RuneLite_2026-10-21_14-03-22.mp4`, `RuneLite_session_2026-10-21_14-03-22 (2).mp4`, …
pub fn is_recording_file(name: &str) -> bool {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"^.+_(session_)?\d{4}-\d{2}-\d{2}_\d{2}-\d{2}-\d{2}( \(\d+\))?\.mp4$").expect("valid regex")
    })
    .is_match(name)
}

/// Every recording under `output_dir` (missing folders just mean none).
pub fn scan(output_dir: &Path) -> Vec<Recording> {
    let mut out = Vec::new();
    let Ok(games) = std::fs::read_dir(output_dir) else { return out };
    for game in games.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.is_dir()) {
        for dir in [game.clone(), game.join("Sessions")] {
            let Ok(entries) = std::fs::read_dir(&dir) else { continue };
            for e in entries.filter_map(|e| e.ok()) {
                let name = e.file_name();
                if !name.to_str().is_some_and(is_recording_file) {
                    continue;
                }
                let Ok(meta) = e.metadata() else { continue };
                if meta.is_file() {
                    out.push(Recording {
                        path: e.path(),
                        size: meta.len(),
                        modified: meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                    });
                }
            }
        }
    }
    out
}

pub fn total_size(recs: &[Recording]) -> u64 {
    recs.iter().map(|r| r.size).sum()
}

/// The recordings to delete, oldest first, to bring the total to at most
/// `limit` bytes. The newest recording is never deleted, even if it alone
/// is over the limit: it's almost always the one that was just saved.
pub fn plan_cleanup(recs: &[Recording], limit: u64) -> Vec<Recording> {
    let mut by_age: Vec<&Recording> = recs.iter().collect();
    by_age.sort_by_key(|r| r.modified);
    let mut total = total_size(recs);
    let mut delete = Vec::new();
    // Everything except the newest is fair game.
    for r in by_age.iter().take(by_age.len().saturating_sub(1)) {
        if total <= limit {
            break;
        }
        total -= r.size;
        delete.push((*r).clone());
    }
    delete
}

/// "4.3 GB", "850 MB".
pub fn format_size(bytes: u64) -> String {
    if bytes >= BYTES_PER_GB {
        format!("{:.1} GB", bytes as f64 / BYTES_PER_GB as f64)
    } else {
        format!("{} MB", bytes / 1_000_000)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn rec(name: &str, size: u64, age_secs: u64) -> Recording {
        Recording {
            path: PathBuf::from(name),
            size,
            modified: SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000 - age_secs),
        }
    }

    #[test]
    fn recognises_only_our_files() {
        assert!(is_recording_file("RuneLite_2026-10-21_14-03-22.mp4"));
        assert!(is_recording_file("Counter-Strike_2_2026-10-21_14-03-22 (2).mp4"));
        assert!(is_recording_file("League_of_Legends_session_2026-10-21_14-03-22.mp4"));
        assert!(!is_recording_file("my edit.mp4"));
        assert!(!is_recording_file("RuneLite_2026-10-21_14-03-22.mp4.concat.txt"));
        assert!(!is_recording_file("RuneLite_2026-10-21_14-03-22.mkv"));
        assert!(!is_recording_file("RuneLite_2026-10-21.mp4"));
    }

    #[test]
    fn deletes_oldest_until_under_limit() {
        let recs = vec![rec("new", 3, 10), rec("oldest", 4, 300), rec("mid", 5, 200), rec("old", 2, 250)];
        // total 14, limit 8: drop oldest (4) → 10, then old (2) → 8.
        let names: Vec<_> = plan_cleanup(&recs, 8).into_iter().map(|r| r.path).collect();
        assert_eq!(names, [PathBuf::from("oldest"), PathBuf::from("old")]);
    }

    #[test]
    fn nothing_to_do_under_limit() {
        let recs = vec![rec("a", 3, 10), rec("b", 4, 20)];
        assert!(plan_cleanup(&recs, 7).is_empty());
        assert!(plan_cleanup(&[], 0).is_empty());
    }

    #[test]
    fn never_deletes_the_newest() {
        // A single recording bigger than the limit survives…
        assert!(plan_cleanup(&[rec("huge", 50, 1)], 10).is_empty());
        // …and with others, everything older goes but the newest stays.
        let recs = vec![rec("huge-new", 50, 1), rec("a", 3, 100), rec("b", 4, 200)];
        let kept: Vec<_> = plan_cleanup(&recs, 10).into_iter().map(|r| r.path).collect();
        assert_eq!(kept, [PathBuf::from("b"), PathBuf::from("a")]);
    }

    #[test]
    fn scan_ignores_foreign_files() {
        let root = std::env::temp_dir().join(format!("ptv-storage-test-{}", std::process::id()));
        let game = root.join("RuneLite");
        std::fs::create_dir_all(game.join("Sessions")).unwrap();
        std::fs::write(game.join("RuneLite_2026-10-21_14-03-22.mp4"), [0u8; 10]).unwrap();
        std::fs::write(game.join("Sessions").join("RuneLite_session_2026-10-21_14-03-22.mp4"), [0u8; 20]).unwrap();
        std::fs::write(game.join("holiday.mp4"), [0u8; 99]).unwrap();
        std::fs::write(root.join("RuneLite_2026-10-21_14-03-22.mp4"), [0u8; 99]).unwrap(); // not in a game folder
        let recs = scan(&root);
        std::fs::remove_dir_all(&root).unwrap();
        assert_eq!(recs.len(), 2);
        assert_eq!(total_size(&recs), 30);
    }

    #[test]
    fn sizes_read_nicely() {
        assert_eq!(format_size(4_300_000_000), "4.3 GB");
        assert_eq!(format_size(850_000_000), "850 MB");
    }
}
