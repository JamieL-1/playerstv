# Quick check before tagging a release (about 2 minutes): playerstv records
# the test game for a short run with no audio, with game audio, and with game
# audio and the microphone mixed, and fails if a clip is missing frames or
# audio tracks. The game stays silent; the microphone run records your mic
# into bench/results (not committed).
#
#   powershell -File scripts/smoke.ps1
#
# Needs the same setup as scripts/bench.ps1: vendor\ffmpeg and ui\dist.
param(
    [int]$Fps = 60,
    [int]$Seconds = 15,
    [string]$Root
)
$ErrorActionPreference = "Stop"
if (-not $Root) { $Root = Split-Path -Parent $PSScriptRoot }

if (-not (Test-Path (Join-Path $Root "vendor\ffmpeg\ffmpeg.exe"))) {
    throw "No vendor\ffmpeg\ffmpeg.exe. Run scripts/fetch-ffmpeg.ps1 first."
}
if (-not (Test-Path (Join-Path $Root "ui\dist\index.html"))) {
    throw "No ui\dist. Build the frontend first: cd ui; npm ci; npm run build"
}
if (Get-Process playerstv -ErrorAction SilentlyContinue) {
    throw "playerstv is running. Quit it from the tray first: it would hold the save hotkey."
}

Push-Location $Root
try {
    cargo build --release --locked -p playerstv -p playerstv-bench
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }
    & (Join-Path $Root "target\release\ptv-bench.exe") smoke --fps $Fps --seconds $Seconds
    if ($LASTEXITCODE -ne 0) { throw "smoke check failed: don't release this build" }
} finally {
    Pop-Location
}
