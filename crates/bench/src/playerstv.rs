//! Runs playerstv under test as a separate instance with its own config, so
//! the real install, its settings and its recordings are never touched.

use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use ptv_common::control::{self, Request};
use ptv_core::config::{Config, GameOverrides};
use ptv_core::profile::{self, GameProfile};

use crate::recorders::Settings;

pub const INSTANCE: &str = "bench";
pub const GAME_NAME: &str = "Bench game";

pub struct Playerstv {
    child: Child,
    pub clips_dir: PathBuf,
}

/// Points this process at the bench instance's control pipe. Call before
/// any other thread starts: changing the environment races with threads
/// reading it.
pub fn use_bench_instance() {
    std::env::set_var("PLAYERSTV_INSTANCE", INSTANCE);
}

/// The config the bench instance runs with: only the test game, no
/// notifications or updates, and the run's settings.
pub fn config(s: &Settings, clips_dir: &Path) -> Config {
    let mut cfg = Config {
        buffer_seconds: s.buffer_seconds,
        output_dir: clips_dir.to_owned(),
        record_full_session: false,
        start_with_windows: false,
        notifications: false,
        auto_update: false,
        storage_limit_gb: None,
        ..Default::default()
    };
    cfg.video.fps = s.fps;
    cfg.audio.game_audio = s.audio;
    cfg.audio.mic = s.mic;
    cfg.audio.mixed_track = s.audio && s.mic;
    cfg.games = profile::builtins();
    for g in &mut cfg.games {
        g.enabled = false;
    }
    cfg.games.push(GameProfile {
        id: "custom-bench-game".into(),
        name: GAME_NAME.into(),
        exe_names: vec!["ptv-bench-game.exe".into()],
        window_title_regex: "^ptv-bench-game$".into(),
        enabled: true,
        overrides: GameOverrides::default(),
    });
    cfg
}

impl Playerstv {
    /// Writes the config into `run_dir` and starts `exe` in the tray.
    pub fn start(exe: &Path, run_dir: &Path, s: &Settings) -> Result<Self> {
        if control::request(Request::Status).is_ok() {
            bail!("a playerstv bench instance is already running; quit it first");
        }
        let clips_dir = run_dir.join("clips");
        std::fs::create_dir_all(&clips_dir)?;
        let config_path = run_dir.join("playerstv.toml");
        let toml = toml::to_string(&config(s, &clips_dir)).context("serialising the bench config")?;
        std::fs::write(&config_path, toml)?;

        let child = Command::new(exe)
            .arg("--background")
            .env("PLAYERSTV_INSTANCE", INSTANCE)
            .env("PLAYERSTV_CONFIG", &config_path)
            .spawn()
            .with_context(|| format!("starting {}", exe.display()))?;
        let me = Self { child, clips_dir };
        me.wait_for(Duration::from_secs(20), "start", |_| true)?;
        Ok(me)
    }

    /// Waits until playerstv reports it's recording the test game.
    pub fn wait_recording(&self, timeout: Duration) -> Result<()> {
        self.wait_for(timeout, "start recording the test game", |snap| snap.status.is_recording())
    }

    fn wait_for(&self, timeout: Duration, what: &str, ok: impl Fn(&control::Snapshot) -> bool) -> Result<()> {
        let deadline = Instant::now() + timeout;
        loop {
            match control::request(Request::Status) {
                Ok(snap) if ok(&snap) => return Ok(()),
                Ok(snap) if Instant::now() >= deadline => bail!("playerstv didn't {what}: {:?}", snap.status),
                Err(e) if Instant::now() >= deadline => bail!("playerstv didn't {what}: {e:#}"),
                _ => std::thread::sleep(Duration::from_millis(200)),
            }
        }
    }

    pub fn stop(mut self) -> Result<()> {
        let _ = control::request(Request::Quit);
        let deadline = Instant::now() + Duration::from_secs(30);
        while Instant::now() < deadline {
            if self.child.try_wait()?.is_some() {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        self.child.kill()?;
        bail!("playerstv didn't quit within 30 s, so it was killed");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bench_config_records_only_the_test_game() {
        let s = Settings {
            width: 2560,
            height: 1440,
            fps: 60,
            buffer_seconds: 30,
            bitrate_mbps: 27,
            audio: false,
            mic: false,
        };
        let cfg = config(&s, Path::new(r"C:\clips"));
        let enabled: Vec<&str> = cfg.games.iter().filter(|g| g.enabled).map(|g| g.name.as_str()).collect();
        assert_eq!(enabled, [GAME_NAME]);
        assert!(!cfg.notifications && !cfg.auto_update && !cfg.start_with_windows && !cfg.audio.mic);
        // Survives the round trip playerstv does when it loads it.
        let back: Config = toml::from_str(&toml::to_string(&cfg).unwrap()).unwrap();
        assert_eq!(back.games.len(), cfg.games.len());
        assert!(back.games.iter().any(|g| g.name == GAME_NAME && g.enabled));
    }
}
