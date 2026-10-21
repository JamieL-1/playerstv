# Downloads a pinned Windows FFmpeg build (needs >= 8.0 for the gfxcapture
# filter, plus NVENC) and puts ffmpeg.exe / ffprobe.exe in vendor/ffmpeg.
#
#   powershell -File scripts/fetch-ffmpeg.ps1                 # into vendor/ffmpeg
#   powershell -File scripts/fetch-ffmpeg.ps1 -Destination X  # somewhere else
#
# The build is Gyan Doshi's "essentials" build, from its GitHub mirror, and
# is checked against the SHA-256 GitHub publishes for it. To upgrade, change
# $Version and $Sha256 together (gh api repos/GyanD/codexffmpeg/releases/tags/<ver>).
param(
    [string]$Destination,
    [string]$Version = "9.0.2",
    [string]$Sha256 = "60f467265b1e312373dbcd92200c2618a74850f98d3d078e94296bb3fa2047ba"
)
$ErrorActionPreference = "Stop"
# Resolved here: Windows PowerShell 5.1 leaves $PSScriptRoot empty inside
# param() defaults.
if (-not $Destination) { $Destination = Join-Path (Split-Path -Parent $PSScriptRoot) "vendor\ffmpeg" }
$ProgressPreference = "SilentlyContinue"  # Invoke-WebRequest is ~10x faster without the progress bar

$name = "ffmpeg-$Version-essentials_build"
$url = "https://github.com/GyanD/codexffmpeg/releases/download/$Version/$name.zip"
$work = Join-Path ([IO.Path]::GetTempPath()) "playerstv-ffmpeg-$Version"
$zip = Join-Path $work "$name.zip"

New-Item -ItemType Directory -Force $work, $Destination | Out-Null
if (-not (Test-Path $zip) -or (Get-FileHash $zip -Algorithm SHA256).Hash -ne $Sha256) {
    Write-Host "Downloading FFmpeg $Version..."
    Invoke-WebRequest $url -OutFile $zip
}
$actual = (Get-FileHash $zip -Algorithm SHA256).Hash
if ($actual -ne $Sha256) {
    Remove-Item $zip -Force
    throw "Checksum mismatch for $url`n  expected $Sha256`n  got      $actual"
}

$extract = Join-Path $work "extract"
if (Test-Path $extract) { Remove-Item -Recurse -Force $extract }
Expand-Archive $zip -DestinationPath $extract
$root = Join-Path $extract $name
Copy-Item (Join-Path $root "bin\ffmpeg.exe"), (Join-Path $root "bin\ffprobe.exe") $Destination -Force
Copy-Item (Join-Path $root "LICENSE") (Join-Path $Destination "LICENSE.txt") -Force

# playerstv captures through gfxcapture; refuse a build without it.
$ffmpeg = Join-Path $Destination "ffmpeg.exe"
$help = & $ffmpeg -hide_banner -h filter=gfxcapture 2>&1 | Out-String
if ($help -notmatch "gfxcapture AVOptions") {
    throw "This FFmpeg build has no gfxcapture filter (need FFmpeg >= 8.0)."
}
Write-Host "FFmpeg $Version installed to $Destination"
