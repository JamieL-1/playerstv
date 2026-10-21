# Downloads a Windows FFmpeg build (>= 8.0, needs the gfxcapture filter and NVENC)
# into vendor/ffmpeg. Run from the repo root:  powershell -File scripts/fetch-ffmpeg.ps1
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
$dest = Join-Path $root "vendor\ffmpeg"
$zip = Join-Path $env:TEMP "playerstv-ffmpeg.zip"
$extract = Join-Path $env:TEMP "playerstv-ffmpeg"

New-Item -ItemType Directory -Force $dest | Out-Null
Write-Host "Downloading FFmpeg (gyan.dev release essentials)..."
Invoke-WebRequest "https://www.gyan.dev/ffmpeg/builds/ffmpeg-release-essentials.zip" -OutFile $zip
if (Test-Path $extract) { Remove-Item -Recurse -Force $extract }
Expand-Archive $zip -DestinationPath $extract
$bin = Get-ChildItem $extract -Recurse -Filter ffmpeg.exe | Select-Object -First 1
Copy-Item $bin.FullName, (Join-Path $bin.DirectoryName "ffprobe.exe") $dest -Force

& (Join-Path $dest "ffmpeg.exe") -hide_banner -h filter=gfxcapture | Select-Object -First 2
if ($LASTEXITCODE -ne 0) { throw "This FFmpeg build has no gfxcapture filter (need FFmpeg >= 8.0)." }
Write-Host "FFmpeg installed to $dest"
