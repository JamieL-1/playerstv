# Bumps the workspace version in Cargo.toml and prints the new version.
#
#   powershell -File scripts/bump-version.ps1 -Bump patch   # 0.1.0 -> 0.1.1
#   powershell -File scripts/bump-version.ps1 -Bump none    # just prints 0.1.0
#
# Only the `version = "..."` line under [workspace.package] is changed; run
# `cargo update --workspace` afterwards to refresh Cargo.lock.
param(
    [ValidateSet("major", "minor", "patch", "none")][string]$Bump = "patch",
    [string]$Path
)
$ErrorActionPreference = "Stop"
# Resolved here: Windows PowerShell 5.1 leaves $PSScriptRoot empty inside
# param() defaults.
if (-not $Path) { $Path = Join-Path (Split-Path -Parent $PSScriptRoot) "Cargo.toml" }

$lines = [IO.File]::ReadAllLines($Path)
$section = ""
$index = -1
for ($i = 0; $i -lt $lines.Length; $i++) {
    if ($lines[$i] -match '^\s*\[([^\]]+)\]') { $section = $Matches[1].Trim() }
    elseif ($section -eq "workspace.package" -and $lines[$i] -match '^\s*version\s*=\s*"(\d+)\.(\d+)\.(\d+)"') {
        $index = $i
        $major, $minor, $patch = [int]$Matches[1], [int]$Matches[2], [int]$Matches[3]
        break
    }
}
if ($index -lt 0) { throw "No version = ""X.Y.Z"" under [workspace.package] in $Path" }

switch ($Bump) {
    "major" { $major++; $minor = 0; $patch = 0 }
    "minor" { $minor++; $patch = 0 }
    "patch" { $patch++ }
}
$version = "$major.$minor.$patch"
if ($Bump -ne "none") {
    $lines[$index] = "version = ""$version"""
    # Keep LF line endings and no BOM, like the rest of the repo.
    [IO.File]::WriteAllText($Path, ($lines -join "`n") + "`n")
}
Write-Output $version
