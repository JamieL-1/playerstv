# Builds the downloadable release zip from an existing release build:
#
#   (cd ui; npm ci; npm run build)   # frontend, embedded in playerstv-ui.exe
#   cargo build --release
#   powershell -File scripts/fetch-ffmpeg.ps1
#   powershell -File scripts/package.ps1 -Version v0.1.0
#
# Produces dist/playerstv-<version>-windows-x64.zip (+ .sha256) containing
# playerstv.exe (tray), playerstv-ui.exe (window), ffmpeg.exe, the README and
# third-party notices. Unzip anywhere and run playerstv.exe - it finds
# ffmpeg.exe and playerstv-ui.exe next to itself.
param(
    [Parameter(Mandatory)][string]$Version,
    [string]$Root,
    [string]$FfmpegDir,
    [string]$OutDir
)
$ErrorActionPreference = "Stop"
# Defaults resolved here: Windows PowerShell 5.1 leaves $PSScriptRoot empty
# inside param() defaults.
if (-not $Root) { $Root = Split-Path -Parent $PSScriptRoot }
if (-not $FfmpegDir) { $FfmpegDir = Join-Path $Root "vendor\ffmpeg" }
if (-not $OutDir) { $OutDir = Join-Path $Root "dist" }

$exe = Join-Path $Root "target\release\playerstv.exe"
$uiExe = Join-Path $Root "target\release\playerstv-ui.exe"
$ffmpeg = Join-Path $FfmpegDir "ffmpeg.exe"
foreach ($f in $exe, $uiExe, $ffmpeg, (Join-Path $FfmpegDir "LICENSE.txt")) {
    if (-not (Test-Path $f)) { throw "Missing $f - build/fetch first (see the comment at the top of this script)." }
}

$name = "playerstv-$Version-windows-x64"
$stage = Join-Path $OutDir $name
if (Test-Path $stage) { Remove-Item -Recurse -Force $stage }
New-Item -ItemType Directory -Force (Join-Path $stage "licenses") | Out-Null

Copy-Item $exe, $uiExe, $ffmpeg $stage
Copy-Item (Join-Path $Root "README.md") $stage
Copy-Item (Join-Path $FfmpegDir "LICENSE.txt") (Join-Path $stage "licenses\FFmpeg-LICENSE.txt")
$ffmpegVersion = (& $ffmpeg -hide_banner -version | Select-Object -First 1)
@"
Third-party software shipped with playerstv
===========================================

ffmpeg.exe
  $ffmpegVersion
  "Essentials" build by Gyan Doshi: https://www.gyan.dev/ffmpeg/builds/
  Licensed under the GNU GPL v3 (see FFmpeg-LICENSE.txt). It is a separate
  program that playerstv runs; FFmpeg source code: https://ffmpeg.org/download.html
"@ | Set-Content -Encoding utf8 (Join-Path $stage "licenses\THIRD-PARTY.txt")

$zip = Join-Path $OutDir "$name.zip"
if (Test-Path $zip) { Remove-Item -Force $zip }
Compress-Archive -Path $stage -DestinationPath $zip -CompressionLevel Optimal
$hash = (Get-FileHash $zip -Algorithm SHA256).Hash.ToLower()
"$hash  $name.zip" | Set-Content -Encoding ascii -NoNewline "$zip.sha256"

Write-Host "Packaged $zip ($([math]::Round((Get-Item $zip).Length / 1MB, 1)) MB)"
Write-Host "SHA-256 $hash"
