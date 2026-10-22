fn main() {
    bundle_ffmpeg();
    tauri_build::build()
}

/// With the `bundle-ffmpeg` feature (release builds), embeds
/// `vendor/ffmpeg/ffmpeg.exe` (from `scripts/fetch-ffmpeg.ps1`) in the exe,
/// keyed by a hash of its contents so a new ffmpeg unpacks to a new folder.
fn bundle_ffmpeg() {
    if std::env::var_os("CARGO_FEATURE_BUNDLE_FFMPEG").is_none() {
        return;
    }
    let manifest = std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let path = manifest.join("..").join("..").join("vendor").join("ffmpeg").join("ffmpeg.exe");
    println!("cargo:rerun-if-changed={}", path.display());
    let bytes = std::fs::read(&path).unwrap_or_else(|e| {
        panic!("bundle-ffmpeg: can't read {} ({e}); run scripts/fetch-ffmpeg.ps1 first", path.display())
    });
    // FNV-1a: only needs to tell ffmpeg builds apart.
    let hash = bytes.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &b| (h ^ b as u64).wrapping_mul(0x0100_0000_01b3));
    println!("cargo:rustc-env=PTV_FFMPEG_PATH={}", path.display());
    println!("cargo:rustc-env=PTV_FFMPEG_KEY={hash:016x}");
}
