use regex::Regex;
use serde::{Deserialize, Serialize};

/// Describes how to recognise a game's window. Adding support for a new game
/// is just adding another profile to the config.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameProfile {
    /// Stable identifier. Built-in games use fixed ids (see [`builtins`]);
    /// custom games get `custom-…`. Older configs without one are fixed up
    /// by [`merge_builtins`].
    #[serde(default)]
    pub id: String,
    /// Display name, also used for the clip sub-folder and file names.
    pub name: String,
    /// Executable file names (case-insensitive) that may own the window.
    pub exe_names: Vec<String>,
    /// Regex the window title must match.
    pub window_title_regex: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

pub const RUNELITE: &str = "runelite";
pub const LEAGUE: &str = "league-of-legends";
pub const CS2: &str = "cs2";

/// Games playerstv knows out of the box. Their detection rules always come
/// from here (only `enabled` is kept from the config), so fixes ship with
/// updates instead of being frozen into old config files.
pub fn builtins() -> Vec<GameProfile> {
    vec![GameProfile::runelite(), GameProfile::league(), GameProfile::cs2()]
}

pub fn is_builtin(id: &str) -> bool {
    [RUNELITE, LEAGUE, CS2].contains(&id)
}

/// Extra context shown next to a built-in game in the UI.
pub fn builtin_note(id: &str) -> Option<&'static str> {
    match id {
        RUNELITE => Some("Old School RuneScape via RuneLite"),
        LEAGUE => Some("Records matches only, not the client/lobby"),
        CS2 => Some("If recordings are black, set Display Mode to Fullscreen Windowed"),
        _ => None,
    }
}

impl GameProfile {
    pub fn runelite() -> Self {
        Self {
            id: RUNELITE.into(),
            name: "RuneLite".into(),
            // RuneLite usually runs as its own launcher exe, but the
            // jar/"--mode=jvm" setups show up as java(w).exe.
            exe_names: vec!["RuneLite.exe".into(), "javaw.exe".into(), "java.exe".into()],
            window_title_regex: r"^RuneLite( - .+)?$".into(),
            enabled: true,
        }
    }

    pub fn league() -> Self {
        Self {
            id: LEAGUE.into(),
            name: "League of Legends".into(),
            // The in-match process. The lobby is LeagueClient*.exe, which we
            // deliberately don't match.
            exe_names: vec!["League of Legends.exe".into()],
            window_title_regex: r"^League of Legends".into(),
            enabled: true,
        }
    }

    pub fn cs2() -> Self {
        Self {
            id: CS2.into(),
            name: "Counter-Strike 2".into(),
            exe_names: vec!["cs2.exe".into()],
            window_title_regex: r"^Counter-Strike".into(),
            enabled: true,
        }
    }

    pub fn is_builtin(&self) -> bool {
        is_builtin(&self.id)
    }

    pub fn matcher(&self) -> Option<ProfileMatcher> {
        Regex::new(&self.window_title_regex).ok().map(|title| ProfileMatcher { profile: self.clone(), title })
    }
}

/// Brings a game list up to date: gives legacy entries ids, refreshes
/// built-in definitions (keeping their on/off state), adds built-ins that are
/// missing, and makes custom ids unique.
pub fn merge_builtins(games: &mut Vec<GameProfile>) {
    let builtins = builtins();
    // Legacy entries (no id) that are a built-in by name become that built-in.
    for g in games.iter_mut().filter(|g| g.id.is_empty()) {
        if let Some(b) = builtins.iter().find(|b| b.name.eq_ignore_ascii_case(g.name.trim())) {
            g.id = b.id.clone();
        }
    }
    // One entry per built-in, with current rules and the user's toggle.
    let mut seen = Vec::new();
    games.retain(|g| {
        !g.is_builtin() || {
            let first = !seen.contains(&g.id);
            seen.push(g.id.clone());
            first
        }
    });
    for g in games.iter_mut().filter(|g| g.is_builtin()) {
        let enabled = g.enabled;
        *g = builtins.iter().find(|b| b.id == g.id).expect("is_builtin").clone();
        g.enabled = enabled;
    }
    // Built-ins first, in their standard order; custom games after.
    for (i, b) in builtins.iter().enumerate() {
        match games.iter().position(|g| g.id == b.id) {
            Some(pos) => {
                let g = games.remove(pos);
                games.insert(i, g);
            }
            None => games.insert(i, b.clone()),
        }
    }
    // Custom games: unique ids.
    let mut used: Vec<String> = games.iter().filter(|g| g.is_builtin()).map(|g| g.id.clone()).collect();
    for g in games.iter_mut().filter(|g| !g.is_builtin()) {
        if g.id.is_empty() || used.contains(&g.id) || !g.id.starts_with("custom-") {
            g.id = unique_custom_id(&g.name, &used);
        }
        used.push(g.id.clone());
    }
}

/// A fresh `custom-…` id not in `used`.
pub fn unique_custom_id(name: &str, used: &[String]) -> String {
    let base = format!("custom-{}", safe_name(name).to_lowercase());
    std::iter::once(base.clone())
        .chain((2..).map(|n| format!("{base}-{n}")))
        .find(|id| !used.contains(id))
        .expect("unbounded")
}

/// A profile with its regex compiled.
#[derive(Debug, Clone)]
pub struct ProfileMatcher {
    pub profile: GameProfile,
    title: Regex,
}

impl ProfileMatcher {
    /// The cheap half of [`Self::matches`]: enabled and the title fits.
    pub fn title_matches(&self, window_title: &str) -> bool {
        self.profile.enabled && self.title.is_match(window_title)
    }

    pub fn matches(&self, exe_name: &str, window_title: &str) -> bool {
        self.profile.enabled
            && self.profile.exe_names.iter().any(|e| e.eq_ignore_ascii_case(exe_name))
            && self.title.is_match(window_title)
    }
}

/// File-system-safe version of a profile name.
pub fn safe_name(name: &str) -> String {
    let s: String =
        name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
    if s.is_empty() {
        "Game".into()
    } else {
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runelite_matches() {
        let m = GameProfile::runelite().matcher().unwrap();
        assert!(m.matches("RuneLite.exe", "RuneLite - on god veng"));
        assert!(m.matches("runelite.EXE", "RuneLite"));
        assert!(m.matches("javaw.exe", "RuneLite - Zezima"));
        assert!(!m.matches("RuneLite.exe", "RuneLite Launcher2"));
        assert!(!m.matches("firefox.exe", "RuneLite - Twitch"));
        assert!(!m.matches("JagexLauncher.exe", "Jagex Launcher"));
    }

    #[test]
    fn league_matches_game_not_client() {
        let m = GameProfile::league().matcher().unwrap();
        assert!(m.matches("League of Legends.exe", "League of Legends (TM) Client"));
        // The lobby/launcher is a different executable with the same title.
        assert!(!m.matches("LeagueClientUx.exe", "League of Legends"));
        assert!(!m.matches("LeagueClient.exe", "League of Legends"));
    }

    #[test]
    fn cs2_matches() {
        let m = GameProfile::cs2().matcher().unwrap();
        assert!(m.matches("cs2.exe", "Counter-Strike 2"));
        assert!(!m.matches("steam.exe", "Counter-Strike 2"));
    }

    #[test]
    fn disabled_never_matches() {
        let mut p = GameProfile::runelite();
        p.enabled = false;
        assert!(!p.matcher().unwrap().matches("RuneLite.exe", "RuneLite"));
    }

    #[test]
    fn merge_upgrades_legacy_config() {
        // A config from before ids existed: RuneLite (disabled, with an old
        // regex) plus a custom game.
        let mut games = vec![
            GameProfile {
                id: String::new(),
                window_title_regex: "^old$".into(),
                enabled: false,
                ..GameProfile::runelite()
            },
            GameProfile {
                id: String::new(),
                name: "Minecraft".into(),
                exe_names: vec!["javaw.exe".into()],
                window_title_regex: "^Minecraft".into(),
                enabled: true,
            },
        ];
        merge_builtins(&mut games);
        let ids: Vec<&str> = games.iter().map(|g| g.id.as_str()).collect();
        assert_eq!(ids, [RUNELITE, LEAGUE, CS2, "custom-minecraft"]);
        assert!(!games[0].enabled, "user's toggle is kept");
        assert_eq!(games[0].window_title_regex, GameProfile::runelite().window_title_regex, "rules refreshed");
        assert!(games[1].enabled && games[2].enabled, "new built-ins start enabled");
    }

    #[test]
    fn merge_is_idempotent_and_dedupes() {
        let mut games = builtins();
        games.push(GameProfile::cs2());
        games.push(GameProfile {
            id: "custom-x".into(),
            name: "X".into(),
            exe_names: vec![],
            window_title_regex: ".*".into(),
            enabled: true,
        });
        games.push(GameProfile {
            id: "custom-x".into(),
            name: "X".into(),
            exe_names: vec![],
            window_title_regex: ".*".into(),
            enabled: true,
        });
        merge_builtins(&mut games);
        let once = games.clone();
        merge_builtins(&mut games);
        assert_eq!(games, once);
        let ids: Vec<&str> = games.iter().map(|g| g.id.as_str()).collect();
        assert_eq!(ids, [RUNELITE, LEAGUE, CS2, "custom-x", "custom-x-2"]);
    }

    #[test]
    fn safe_names() {
        assert_eq!(safe_name("RuneLite"), "RuneLite");
        assert_eq!(safe_name("Old School: RS"), "Old_School__RS");
    }
}
