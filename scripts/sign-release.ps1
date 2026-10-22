# Prepares the self-update files for a release, from an existing release
# build (see package.ps1 for building it):
#
#   powershell -File scripts/sign-release.ps1 -Version v0.1.1 -KeyFile path\to\minisign.key
#
# Produces in dist/:
#   playerstv-<version>-windows-x64.exe          the exe the updater downloads
#   playerstv-<version>-windows-x64.exe.minisig  its signature by the release key
#   latest.json                                   what the app checks for updates
#
# The key is a minisign secret key without a password (minisign -G -W); the
# app only installs files signed by the matching PUBLIC_KEY in
# crates/app/src/updater.rs, with "playerstv <version>" as the signed comment.
param(
    [Parameter(Mandatory)][string]$Version,
    [Parameter(Mandatory)][string]$KeyFile,
    [string]$Minisign = "minisign",
    # Where the release's files will be downloadable from.
    [string]$BaseUrl,
    [string]$Root,
    [string]$OutDir
)
$ErrorActionPreference = "Stop"
# Defaults resolved here: Windows PowerShell 5.1 leaves $PSScriptRoot empty
# inside param() defaults.
if (-not $Root) { $Root = Split-Path -Parent $PSScriptRoot }
if (-not $OutDir) { $OutDir = Join-Path $Root "dist" }
if (-not $BaseUrl) { $BaseUrl = "https://github.com/JamieL-1/playerstv/releases/download/$Version" }

$bare = $Version.TrimStart("v")
$exe = Join-Path $Root "target\release\playerstv.exe"
if (-not (Test-Path $exe)) { throw "Missing $exe - build it first (see package.ps1)." }

$name = "playerstv-$Version-windows-x64.exe"
$asset = Join-Path $OutDir $name
New-Item -ItemType Directory -Force $OutDir | Out-Null
Copy-Item $exe $asset -Force

& $Minisign -S -s $KeyFile -m $asset -x "$asset.minisig" -t "playerstv $bare"
if ($LASTEXITCODE -ne 0) { throw "minisign failed to sign $asset" }

# Check against the key the app trusts, so a mismatched secret fails here
# rather than on every tester's machine.
$updater = Get-Content -Raw (Join-Path $Root "crates\app\src\updater.rs")
if ($updater -notmatch 'const PUBLIC_KEY: &str = "([^"]+)"') { throw "PUBLIC_KEY not found in updater.rs" }
& $Minisign -V -q -m $asset -x "$asset.minisig" -P $Matches[1]
if ($LASTEXITCODE -ne 0) { throw "The signature doesn't verify with updater.rs's PUBLIC_KEY - wrong key file?" }

$manifest = [ordered]@{
    version = $bare
    url     = "$BaseUrl/$name"
    sha256  = (Get-FileHash $asset -Algorithm SHA256).Hash.ToLower()
    minisig = [IO.File]::ReadAllText("$asset.minisig")
}
# UTF-8 without a BOM, which Set-Content in PowerShell 5.1 can't do.
[IO.File]::WriteAllText((Join-Path $OutDir "latest.json"), ($manifest | ConvertTo-Json), (New-Object Text.UTF8Encoding $false))

Write-Host "Signed $asset"
Write-Host "Wrote $(Join-Path $OutDir 'latest.json') -> $($manifest.url)"
