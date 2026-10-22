//! Self-update. Release builds fetch `latest.json` from the newest GitHub
//! release; if it names a newer version, the new `playerstv.exe` is
//! downloaded in the background, checked (SHA-256 and a minisign signature by
//! the release key) and handed to the tray as a [`Staged`] update.
//!
//! [`install`] swaps it in for the running exe: Windows lets a running exe be
//! renamed but not overwritten, so ours becomes `playerstv.old.exe` and the
//! new one takes its name. The tray then restarts (see `tray.rs`), or the new
//! version simply starts next time. [`finish_previous`] removes the old exe.
//!
//! Test builds: `PLAYERSTV_UPDATE_URL` points at another `latest.json` and
//! turns updates on even for a `PLAYERSTV_INSTANCE`.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, ensure, Context, Result};
use minisign_verify::{PublicKey, Signature};
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::paths;

const MANIFEST_URL: &str = "https://github.com/JamieL-1/playerstv/releases/latest/download/latest.json";
/// The release signing key (`scripts/sign-release.ps1` signs with its secret
/// half, kept in the `MINISIGN_KEY` Actions secret).
const PUBLIC_KEY: &str = "RWTLoi2nezNS9eiBO+1OkebxdiHEeqJgun3r8+25tsbltgezLINnfPc/";
/// A tray left running for days (Start with Windows) still picks up releases.
const RECHECK: Duration = Duration::from_secs(6 * 3600);
/// playerstv.exe is ~120 MB with ffmpeg inside; anything far bigger is wrong.
const MAX_DOWNLOAD: u64 = 1024 * 1024 * 1024;

/// `latest.json`, written by `scripts/sign-release.ps1`.
#[derive(Debug, Clone, Deserialize)]
pub struct Manifest {
    /// "0.1.2" (no "v").
    pub version: String,
    /// Download URL of the new playerstv.exe.
    pub url: String,
    pub sha256: String,
    /// Contents of the exe's `.minisig` file.
    pub minisig: String,
}

/// A downloaded and verified update, ready for [`install`].
#[derive(Debug, Clone)]
pub struct Staged {
    pub version: String,
    pub exe: PathBuf,
}

/// Release builds update themselves; dev builds and separate instances
/// (`PLAYERSTV_INSTANCE`) don't, unless `PLAYERSTV_UPDATE_URL` asks for it.
pub fn enabled() -> bool {
    std::env::var_os("PLAYERSTV_UPDATE_URL").is_some() || (cfg!(feature = "auto-update") && paths::instance().is_none())
}

fn manifest_url() -> String {
    std::env::var("PLAYERSTV_UPDATE_URL").unwrap_or_else(|_| MANIFEST_URL.to_owned())
}

/// Checks now and then every [`RECHECK`] while `allowed` (the `auto_update`
/// setting) is on, and calls `on_ready` once with the first verified update.
pub fn spawn(allowed: Arc<AtomicBool>, on_ready: impl FnOnce(Staged) + Send + 'static) {
    let spawned = std::thread::Builder::new().name("updater".into()).spawn(move || loop {
        if !allowed.load(Ordering::Relaxed) {
            // Switching updates back on should take effect soon, not in hours.
            std::thread::sleep(Duration::from_secs(60));
            continue;
        }
        match check_and_stage() {
            Ok(Some(staged)) => {
                tracing::info!("update {} downloaded to {}", staged.version, staged.exe.display());
                return on_ready(staged);
            }
            Ok(None) => {}
            Err(e) => tracing::warn!("update check failed: {e:#}"),
        }
        std::thread::sleep(RECHECK);
    });
    if let Err(e) = spawned {
        tracing::error!("couldn't start the updater: {e}");
    }
}

fn agent(timeout: Duration) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(15)))
        .timeout_global(Some(timeout))
        .user_agent(concat!("playerstv/", env!("CARGO_PKG_VERSION")))
        .build()
        .new_agent()
}

fn check_and_stage() -> Result<Option<Staged>> {
    let url = manifest_url();
    let manifest: Manifest = agent(Duration::from_secs(30))
        .get(&url)
        .call()
        .with_context(|| format!("fetching {url}"))?
        .body_mut()
        .read_json()
        .context("reading latest.json")?;
    if !is_newer(&manifest.version, env!("CARGO_PKG_VERSION"))? {
        tracing::info!("up to date (latest release is {})", manifest.version);
        return Ok(None);
    }
    tracing::info!("downloading update {}", manifest.version);
    stage(&manifest, &paths::update_dir()).map(Some)
}

fn is_newer(latest: &str, current: &str) -> Result<bool> {
    let parse =
        |v: &str| semver::Version::parse(v.trim_start_matches('v')).with_context(|| format!("bad version {v:?}"));
    Ok(parse(latest)? > parse(current)?)
}

/// Downloads the manifest's exe into `dir` (reusing an earlier complete
/// download) and verifies it. Other files in `dir` are removed.
fn stage(manifest: &Manifest, dir: &Path) -> Result<Staged> {
    std::fs::create_dir_all(dir)?;
    let exe = dir.join(format!("playerstv-{}.exe", manifest.version));
    if !(exe.exists() && verify(&exe, manifest).is_ok()) {
        let part = exe.with_extension("exe.part");
        download(&manifest.url, &part)?;
        if let Err(e) = verify(&part, manifest) {
            let _ = std::fs::remove_file(&part);
            return Err(e);
        }
        std::fs::rename(&part, &exe)?;
    }
    for entry in std::fs::read_dir(dir)?.flatten() {
        if entry.path() != exe {
            let _ = std::fs::remove_file(entry.path());
        }
    }
    Ok(Staged { version: manifest.version.clone(), exe })
}

fn download(url: &str, to: &Path) -> Result<()> {
    let response = agent(Duration::from_secs(60 * 60)).get(url).call().with_context(|| format!("downloading {url}"))?;
    let mut body = response.into_body().into_with_config().limit(MAX_DOWNLOAD).reader();
    let mut file = std::fs::File::create(to).with_context(|| format!("creating {}", to.display()))?;
    std::io::copy(&mut body, &mut file).with_context(|| format!("downloading {url}"))?;
    file.flush()?;
    Ok(())
}

/// The file must match the manifest's SHA-256 and carry a signature by the
/// release key whose signed comment names the same version, so an older
/// signed exe can't be passed off as a newer one.
fn verify(file: &Path, manifest: &Manifest) -> Result<()> {
    verify_with_key(file, manifest, PUBLIC_KEY)
}

fn verify_with_key(file: &Path, manifest: &Manifest, public_key: &str) -> Result<()> {
    let key = PublicKey::from_base64(public_key).context("bad public key")?;
    let signature = Signature::decode(&manifest.minisig).context("bad signature")?;
    ensure!(
        signature.trusted_comment() == format!("playerstv {}", manifest.version),
        "signature is for {:?}, not playerstv {}",
        signature.trusted_comment(),
        manifest.version
    );
    let mut verifier = key.verify_stream(&signature).context("signature doesn't match the release key")?;
    let mut sha = Sha256::new();
    let mut input = std::fs::File::open(file).with_context(|| format!("opening {}", file.display()))?;
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = input.read(&mut buf)?;
        if n == 0 {
            break;
        }
        verifier.update(&buf[..n]);
        sha.update(&buf[..n]);
    }
    let sha = format!("{:x}", sha.finalize());
    if !sha.eq_ignore_ascii_case(manifest.sha256.trim()) {
        bail!("SHA-256 mismatch: got {sha}, expected {}", manifest.sha256);
    }
    verifier.finalize().context("signature check failed")?;
    Ok(())
}

/// Replaces the running exe with the staged one. On error the running exe is
/// left (or put back) where it was.
pub fn install(staged: &Staged) -> Result<()> {
    let exe = std::env::current_exe().context("finding our own exe")?;
    install_over(&staged.exe, &exe)?;
    tracing::info!("installed {} over {}", staged.version, exe.display());
    Ok(())
}

fn install_over(new: &Path, exe: &Path) -> Result<()> {
    let dir = exe.parent().context("exe has no folder")?;
    let stem = exe.file_stem().unwrap_or_default().to_string_lossy();
    // Copied next to the exe first: the staged copy may be on another drive,
    // and a rename on the same one can't half-happen.
    let incoming = dir.join(format!("{stem}.new.exe"));
    std::fs::copy(new, &incoming).with_context(|| format!("copying the update to {}", incoming.display()))?;
    let mut old = dir.join(format!("{stem}.old.exe"));
    if std::fs::remove_file(&old).is_err() && old.exists() {
        // An earlier old exe that's still running.
        old = dir.join(format!("{stem}.old-{}.exe", std::process::id()));
    }
    if let Err(e) = std::fs::rename(exe, &old) {
        let _ = std::fs::remove_file(&incoming);
        return Err(e).with_context(|| format!("moving {} aside", exe.display()));
    }
    if let Err(e) = std::fs::rename(&incoming, exe) {
        let _ = std::fs::rename(&old, exe);
        let _ = std::fs::remove_file(&incoming);
        return Err(e).with_context(|| format!("putting the update at {}", exe.display()));
    }
    Ok(())
}

/// Run at startup: removes what an update left behind (the old exe, the
/// download). Returns true on the first start of a freshly installed update:
/// there's an old exe, and the download folder it came from is still there
/// (it's removed here, so an old exe that can't be deleted yet doesn't make
/// every later start look like an update too). The old processes may take a
/// moment to exit, so removal is retried in the background.
pub fn finish_previous() -> bool {
    let Ok(exe) = std::env::current_exe() else { return false };
    let olds = leftovers(&exe);
    if olds.is_empty() {
        return false;
    }
    let just_updated = std::fs::remove_dir_all(paths::update_dir()).is_ok();
    std::thread::spawn(move || {
        for _ in 0..30 {
            if olds.iter().all(|p| std::fs::remove_file(p).is_ok() || !p.exists()) {
                return;
            }
            std::thread::sleep(Duration::from_secs(2));
        }
        tracing::warn!("couldn't remove {olds:?}");
    });
    just_updated
}

fn leftovers(exe: &Path) -> Vec<PathBuf> {
    let (Some(dir), Some(stem)) = (exe.parent(), exe.file_stem()) else { return Vec::new() };
    let prefix = format!("{}.old", stem.to_string_lossy()).to_lowercase();
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            let name = p.file_name().unwrap_or_default().to_string_lossy().to_lowercase();
            name.starts_with(&prefix) && name.ends_with(".exe")
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // Made with `minisign -G -W` and
    // `minisign -S -m payload.bin -t "playerstv 9.9.9"`, payload b"update".
    const TEST_KEY: &str = "RWTsquJEX7AK7DSWjLgQjIwAQdECQbyZ5uXXMHg6e+rksba/ILh+iY4p";
    const TEST_SIG: &str = "untrusted comment: signature from minisign secret key
RUTsquJEX7AK7KryeLqVMHWqpCAvqzMC9hRyvRLPjT81NIlCbgywKF/NbeiI1Hv4h8Cy+07qVJPaWmF5cx2SZjmJYRULFkttPAY=
trusted comment: playerstv 9.9.9
3we8j7Zmg9TGfyle38MKpJbiui+TExRCA12UULBbfnDhUSFGD4rANtiPcwjH/2LFnUzCVGRAsoIq+QdCHJhgDQ==
";
    const PAYLOAD_SHA: &str = "2937013f2181810606b2a799b05bda2849f3e369a20982a4138f0e0a55984ce4";

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ptv-update-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn manifest(version: &str, sha256: &str) -> Manifest {
        Manifest { version: version.into(), url: String::new(), sha256: sha256.into(), minisig: TEST_SIG.into() }
    }

    #[test]
    fn compares_versions() {
        assert!(is_newer("0.1.1", "0.1.0").unwrap());
        assert!(is_newer("v0.2.0", "0.1.9").unwrap());
        assert!(is_newer("0.10.0", "0.9.0").unwrap(), "numeric, not text, order");
        assert!(!is_newer("0.1.0", "0.1.0").unwrap());
        assert!(!is_newer("0.1.0", "0.2.0").unwrap(), "never downgrades");
        assert!(is_newer("garbage", "0.1.0").is_err());
    }

    #[test]
    fn parses_manifest() {
        let m: Manifest =
            serde_json::from_str(r#"{"version":"0.1.1","url":"https://x/p.exe","sha256":"ab","minisig":"sig"}"#)
                .unwrap();
        assert_eq!(m.version, "0.1.1");
        assert_eq!(m.url, "https://x/p.exe");
    }

    #[test]
    fn verifies_hash_signature_and_version() {
        let dir = temp_dir("verify");
        let file = dir.join("payload.bin");
        std::fs::write(&file, b"update").unwrap();
        verify_with_key(&file, &manifest("9.9.9", PAYLOAD_SHA), TEST_KEY).unwrap();

        // A signature for 9.9.9 can't vouch for another version.
        assert!(verify_with_key(&file, &manifest("9.9.10", PAYLOAD_SHA), TEST_KEY).is_err());
        // Wrong hash.
        assert!(verify_with_key(&file, &manifest("9.9.9", &"0".repeat(64)), TEST_KEY).is_err());
        // Tampered file with a hash that matches it: the signature still fails.
        std::fs::write(&file, b"evil!!").unwrap();
        let evil_sha = format!("{:x}", Sha256::digest(b"evil!!"));
        assert!(verify_with_key(&file, &manifest("9.9.9", &evil_sha), TEST_KEY).is_err());
        // Signed by someone else.
        std::fs::write(&file, b"update").unwrap();
        assert!(verify(&file, &manifest("9.9.9", PAYLOAD_SHA)).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn installs_over_the_exe_and_cleans_up() {
        let dir = temp_dir("install");
        let exe = dir.join("playerstv.exe");
        let new = dir.join("staged.exe");
        std::fs::write(&exe, b"v1").unwrap();
        std::fs::write(&new, b"v2").unwrap();
        // Stands in for an old exe that's still running and can't be deleted.
        let busy = dir.join("playerstv.old.exe");
        std::fs::write(&busy, b"v0").unwrap();
        // No FILE_SHARE_DELETE, like a running exe's image.
        let lock = {
            use std::os::windows::fs::OpenOptionsExt;
            std::fs::OpenOptions::new().read(true).share_mode(1 /* FILE_SHARE_READ */).open(&busy).unwrap()
        };

        install_over(&new, &exe).unwrap();
        assert_eq!(std::fs::read(&exe).unwrap(), b"v2");
        assert!(!dir.join("playerstv.new.exe").exists());
        let mut olds = leftovers(&exe);
        olds.sort();
        assert_eq!(olds.len(), 2, "{olds:?}");
        assert!(olds.iter().any(|p| std::fs::read(p).unwrap() == b"v1"));
        drop(lock);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn failed_install_keeps_the_exe() {
        let dir = temp_dir("install-fail");
        let exe = dir.join("playerstv.exe");
        std::fs::write(&exe, b"v1").unwrap();
        assert!(install_over(&dir.join("missing.exe"), &exe).is_err());
        assert_eq!(std::fs::read(&exe).unwrap(), b"v1");
        assert!(leftovers(&exe).is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
