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

/// `RuneLite_2026-10-21_14-03-22.mp4`, `RuneLite_session_2026-10-21_14-03-22 (2).mp4`,
/// `Big fight_2026-10-21_14-03-22.mp4`, …
pub fn is_recording_file(name: &str) -> bool {
    split_name(name).is_some()
}

/// A recording's file name as (label, timestamp): `Big fight_2026-10-21_14-03-22 (2).mp4`
/// → `("Big fight", "2026-10-21_14-03-22")`. The label of an unnamed one is
/// `<Game>` or `<Game>_session`.
pub fn split_name(name: &str) -> Option<(&str, &str)> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(r"^(.+)_(\d{4}-\d{2}-\d{2}_\d{2}-\d{2}-\d{2})( \(\d+\))?\.mp4$").expect("valid regex")
    });
    let caps = re.captures(name)?;
    Some((caps.get(1)?.as_str(), caps.get(2)?.as_str()))
}

/// The name the user gave the recording at `path`, if any: its label,
/// unless that's the default for its game folder.
pub fn custom_name(path: &Path) -> Option<&str> {
    let (label, _) = split_name(path.file_name()?.to_str()?)?;
    let parent = path.parent()?;
    let game_dir = if parent.file_name()? == "Sessions" { parent.parent()? } else { parent };
    let game = game_dir.file_name()?.to_str()?;
    let default = label == game || label.strip_suffix("_session") == Some(game);
    (!default).then_some(label)
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
/// is over the limit: it's almost always the one that was just saved. With
/// `keep_named`, neither are recordings the user named (see [`custom_name`]),
/// though they still count toward the total.
pub fn plan_cleanup(recs: &[Recording], limit: u64, keep_named: bool) -> Vec<Recording> {
    let mut by_age: Vec<&Recording> = recs.iter().collect();
    by_age.sort_by_key(|r| r.modified);
    let mut total = total_size(recs);
    let mut delete = Vec::new();
    // Everything except the newest is fair game.
    for r in by_age.iter().take(by_age.len().saturating_sub(1)) {
        if total <= limit {
            break;
        }
        if keep_named && custom_name(&r.path).is_some() {
            continue;
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
        assert!(is_recording_file("Big fight!_2026-10-21_14-03-22.mp4"));
    }

    #[test]
    fn names_split_into_label_and_timestamp() {
        assert_eq!(split_name("RuneLite_2026-10-21_14-03-22.mp4"), Some(("RuneLite", "2026-10-21_14-03-22")));
        assert_eq!(
            split_name("RuneLite_session_2026-10-21_14-03-22 (2).mp4"),
            Some(("RuneLite_session", "2026-10-21_14-03-22"))
        );
        assert_eq!(split_name("Big_fight_2026-10-21_14-03-22.mp4"), Some(("Big_fight", "2026-10-21_14-03-22")));
        assert_eq!(split_name("holiday.mp4"), None);
    }

    #[test]
    fn custom_names_are_labels_other_than_the_default() {
        let game = Path::new(r"C:\out\RuneLite");
        assert_eq!(custom_name(&game.join("RuneLite_2026-10-21_14-03-22.mp4")), None);
        assert_eq!(custom_name(&game.join("Sessions").join("RuneLite_session_2026-10-21_14-03-22.mp4")), None);
        assert_eq!(custom_name(&game.join("Big fight_2026-10-21_14-03-22 (2).mp4")), Some("Big fight"));
        assert_eq!(custom_name(&game.join("Sessions").join("Raid night_2026-10-21_14-03-22.mp4")), Some("Raid night"));
    }

    #[test]
    fn keep_named_spares_named_recordings() {
        let named = |n: &str, size, age| rec(&format!(r"C:\out\RuneLite\{n}_2026-10-21_14-03-22.mp4"), size, age);
        let recs = vec![named("RuneLite", 3, 10), named("Keeper", 4, 300), named("RuneLite", 5, 200)];
        // total 12, limit 7: the oldest is named, so the next oldest goes instead.
        let deleted: Vec<_> = plan_cleanup(&recs, 7, true).into_iter().map(|r| r.size).collect();
        assert_eq!(deleted, [5]);
        let deleted: Vec<_> = plan_cleanup(&recs, 7, false).into_iter().map(|r| r.size).collect();
        assert_eq!(deleted, [4, 5]);
    }

    #[test]
    fn deletes_oldest_until_under_limit() {
        let recs = vec![rec("new", 3, 10), rec("oldest", 4, 300), rec("mid", 5, 200), rec("old", 2, 250)];
        // total 14, limit 8: drop oldest (4) → 10, then old (2) → 8.
        let names: Vec<_> = plan_cleanup(&recs, 8, false).into_iter().map(|r| r.path).collect();
        assert_eq!(names, [PathBuf::from("oldest"), PathBuf::from("old")]);
    }

    #[test]
    fn nothing_to_do_under_limit() {
        let recs = vec![rec("a", 3, 10), rec("b", 4, 20)];
        assert!(plan_cleanup(&recs, 7, false).is_empty());
        assert!(plan_cleanup(&[], 0, false).is_empty());
    }

    #[test]
    fn never_deletes_the_newest() {
        // A single recording bigger than the limit survives…
        assert!(plan_cleanup(&[rec("huge", 50, 1)], 10, false).is_empty());
        // …and with others, everything older goes but the newest stays.
        let recs = vec![rec("huge-new", 50, 1), rec("a", 3, 100), rec("b", 4, 200)];
        let kept: Vec<_> = plan_cleanup(&recs, 10, false).into_iter().map(|r| r.path).collect();
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
