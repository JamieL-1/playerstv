# Runs the recorder benchmark (see BENCHMARKS.md) and writes the report:
#
#   powershell -File scripts/bench.ps1                          # playerstv vs no recorder
#   powershell -File scripts/bench.ps1 -Recorders playerstv,obs,nvidia -Rounds 5
#   powershell -File scripts/bench.ps1 -Audio                   # adds audio and A/V sync (audible)
#
# Builds release binaries first: debug builds are meaningless to measure.
# Switches to the High performance power plan for the run and puts the
# previous plan back afterwards. Results land in bench/results/<date-time>,
# with report.md at the top.
#
# Manual recorders (OBS, NVIDIA, Medal, Outplayed, Game Bar) print a setup
# checklist and wait for Enter before each run. Pass -ClipsDir obs=D:\clips
# for any that don't save to their default folder.
param(
    [string[]]$Recorders = @("playerstv"),
    [int]$Rounds = 5,
    [int]$Warmup = 30,
    [int]$Seconds = 180,
    [int]$Fps = 60,
    [int]$GpuLoad = 0,
    [string]$Windowed,
    [switch]$Audio,
    [switch]$Idle,
    [string[]]$ClipsDir = @(),
    [string[]]$Hotkey = @(),
    [string]$Out,
    [string]$Root
)
$ErrorActionPreference = "Stop"
# Resolved here: Windows PowerShell 5.1 leaves $PSScriptRoot empty inside
# param() defaults.
if (-not $Root) { $Root = Split-Path -Parent $PSScriptRoot }

if (-not (Test-Path (Join-Path $Root "vendor\ffmpeg\ffmpeg.exe"))) {
    throw "No vendor\ffmpeg\ffmpeg.exe. Run scripts/fetch-ffmpeg.ps1 first."
}
if (-not (Test-Path (Join-Path $Root "ui\dist\index.html"))) {
    throw "No ui\dist. Build the frontend first: cd ui; npm ci; npm run build"
}
if (Get-Process playerstv -ErrorAction SilentlyContinue) {
    throw "playerstv is running. Quit it from the tray first: it would hold the save hotkey and skew the numbers."
}

Push-Location $Root
try {
    cargo build --release --locked -p playerstv -p playerstv-bench
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }

    $benchArgs = @("suite", "--rounds", $Rounds, "--warmup", $Warmup, "--seconds", $Seconds, "--fps", $Fps,
        "--gpu-load", $GpuLoad, "--recorders", ($Recorders -join ","))
    if ($Windowed) { $benchArgs += @("--windowed", $Windowed) }
    if ($Audio) { $benchArgs += "--audio" }
    if ($Idle) { $benchArgs += "--idle" }
    if ($Out) { $benchArgs += @("--out", $Out) }
    foreach ($c in $ClipsDir) { $benchArgs += @("--clips-dir", $c) }
    foreach ($h in $Hotkey) { $benchArgs += @("--hotkey", $h) }
    $manual = $Recorders | Where-Object { $_ -notin @("playerstv", "none") }
    if (-not $manual) { $benchArgs += "--yes" }

    # GUID of the active plan, from "Power Scheme GUID: <guid>  (Name)".
    $previousPlan = ((powercfg /getactivescheme) -split '\s+')[3]
    powercfg /setactive SCHEME_MIN | Out-Null
    try {
        & (Join-Path $Root "target\release\ptv-bench.exe") @benchArgs
        if ($LASTEXITCODE -ne 0) { throw "the benchmark reported failures (see failures.txt in the results folder)" }
    } finally {
        powercfg /setactive $previousPlan | Out-Null
    }
} finally {
    Pop-Location
}
