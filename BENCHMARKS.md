# Benchmarks

playerstv aims to cost players less than any other replay recorder. This is
how that claim is measured, so anyone can check it on their own machine.

## What gets measured

Every recorder records the same thing: a test game, `ptv-bench-game`, which
draws a fixed picture sequence at a fixed frame rate. Each frame is a pure
function of its number, and the number is drawn into the top of the frame.
That makes three checks possible:

- **Which frames a clip holds.** The analyzer reads every frame's number, so
  dropped and repeated frames are counted exactly, not estimated.
- **How close the clip is to what the game drew.** The analyzer redraws each
  frame it finds and compares them with SSIM and PSNR.
- **Audio/video sync**, with `--audio`. The game plays a quiet tone with a
  click at the start of every second and flashes a square on the same frame.
  The clicks take turns being 20, 40, 60 and 80 ms long, so sound that's a
  whole second late can't pass as in sync.

While a recorder runs, `ptv-bench` samples once a second:

| Metric | Source |
|---|---|
| CPU, RAM and disk writes of the recorder's processes (and their children, e.g. playerstv's ffmpeg) | `GetProcessTimes`, `GetProcessMemoryInfo`, `GetProcessIoCounters` |
| GPU 3D and video encode use, per process and system-wide | `\GPU Engine(*)\Utilization Percentage` |
| GPU memory | `\GPU Adapter Memory(*)\Dedicated Usage`, `\GPU Process Memory(*)\Dedicated Usage` |
| Whole-system CPU and disk writes | `GetSystemTimes`, `\PhysicalDisk(_Total)\Disk Write Bytes/sec` |
| How smoothly the game runs | The game's own log of when each frame was presented |
| Save time | From pressing the save hotkey to the clip being complete on disk |

Some recorders do their work in drivers or Windows services rather than in
their own processes, so the main comparison uses whole-system numbers minus
a **baseline** run with no recorder in the same round.

## Fair comparison

- Every recorder records the same window, at the same resolution and frame
  rate, with a 30 s buffer, H.264 on NVENC, and a matched peak bitrate where
  it can be set (the cap playerstv picks for High quality). The harness
  prints a setup checklist for each recorder it can't configure itself.
- Every save is triggered by the recorder's real hotkey, pressed the same
  way.
- Every round runs each recorder once plus the baseline, in a shuffled
  order, so heat and background activity don't always land on the same one.
- Each run warms up for 30 s, then measures for 180 s. The numbers are the
  per-run median, then the median across rounds with a 95% bootstrap
  interval.
- Only release builds are measured.
- No other recorder may be running. Before each run the harness waits for
  the previous one to be closed (or, with `--yes`, fails the run).
- A report only combines runs made with the same settings on the same
  machine, driver and build, and a suite needs an empty results folder.

## Pass/fail rules

These were set before any competitor was measured.

A run only counts if its clip:

- has every frame identifiable;
- is at least as long as the buffer;
- with `--audio`: has an audio track, with sound within 45 ms of the
  picture.

Runs that fail are listed in the report and left out of the numbers.

Dropped and repeated frames are compared as metrics (the share of the frames
the game drew), not used to exclude runs. The first draft required 0 dropped
frames, but playerstv's own clips failed it (see below), and so would any
recorder built on the same Windows capture API. Excluding them all would
leave nothing to compare.

## Known issue: dropped frames

At 60 fps, playerstv's clips can miss frames, each one replaced by a repeat
of the previous frame. How many depends on what else is running: 3 or 4 per
30 s clip with other apps open, but around 40% with the desktop otherwise
idle. What was found so far:

- Windows Graphics Capture, which playerstv uses through FFmpeg's
  `gfxcapture`, delivered only about 40 of the test game's 60 frames per
  second. That held with no FFmpeg involved, with 2 or 8 frame buffers, any
  minimum update interval, and at idle or full GPU clocks.
- playerstv's 60 fps limit adds drops of its own: `max_framerate` throws
  away frames that arrive slightly early, and a fixed 60 fps output puts
  frames near a slot boundary into the same slot. Encoding each new frame
  once with its own timestamp (variable frame rate) would fix this part.
- Desktop Duplication (`ddagrab`) saw all 60 frames of a windowed game, but
  only about 43 when the game ran fullscreen.

The first full run ([report](bench/published/2026-10-26-rtx5080-1440p60.md),
2560×1440 fullscreen) lost 34% of frames. Its audio/video sync also varied
between runs, with sound 8 to 113 ms early, which may be linked to the same
capture timing.

Still open: whether Windows shows all 60 frames on screen in that state
(PresentMon would tell), and which capture method and frame timing
playerstv should use.

**Update, 2026-10-27** (short single rounds, so indicative only):

- The fixed 60 fps output turned every frame that arrived late into a
  repeat. playerstv now keeps each frame's capture time (variable frame
  rate): repeats went to 0%, dropped frames to 27%.
- The rest is lost in capture. Windows Graphics Capture delivered about
  73% of the frames of a game that fills its monitor and is the only
  window on it, as a game started from a launcher is, at every capture
  setting and with no encoder at all. With another window open (a
  console, for example) it delivered about 90%, which made some earlier
  tests look better than real use.
- Desktop Duplication delivered every frame, but records the whole
  monitor, so it could catch other windows and notifications. playerstv
  only ever records the game window, so it isn't used.

- It's Windows, not ffmpeg. `ptv-wgc-probe` captures the game window with
  Windows Graphics Capture directly, without ffmpeg, and reads every
  frame's index. It kept 68 to 71% of frames whether the frame pool held 2
  buffers or 8, and 61% with a 1 ms minimum update interval, against 74 to
  76% for ffmpeg's `gfxcapture`. Every frame it got was a new one: Windows
  never delivers the rest. The same held with the game in a window.
- The test game is now a windowed (GUI) program like a real game. As a
  console program with a console window of its own it was captured at
  about 90%, which no player would see.

So without injecting into the game, recording only its window keeps about
three in four frames of a 60 fps game on this machine.

The A/V offset came from the two streams starting their clocks at different
moments: video at the first captured frame, audio when its feeder finished
opening WASAPI (up to about 100 ms later for one game's sound). Both now
count from one wall-clock origin picked before ffmpeg starts.

**playerstv is better on a metric** when the 95% interval of the difference
(playerstv minus the other recorder) lies entirely on the better side of
zero. If it straddles zero, the report says there's no measurable
difference.

## Running it

You need an NVIDIA GPU, FFmpeg in `vendor/ffmpeg` (`scripts/fetch-ffmpeg.ps1`)
and the built frontend (`cd ui; npm ci; npm run build`). Quit playerstv and
any other recorder first.

```powershell
# playerstv against no recorder, 5 rounds (about 40 minutes)
powershell -File scripts/bench.ps1

# Against other recorders: each one prints its setup steps and waits for Enter
powershell -File scripts/bench.ps1 -Recorders playerstv,obs,nvidia,medal,outplayed,gamebar -Idle

# Add audio capture and A/V sync. The test game plays a quiet tone.
powershell -File scripts/bench.ps1 -Audio
```

The test game covers the primary monitor, like a borderless fullscreen game.
`-Windowed 1920x1080` runs it in a window instead.

Results go to `bench/results/<date-time>/`: one JSON file per run, the saved
clips, and `report.md`. Rebuild a report with
`target\release\ptv-bench.exe report <folder>`. Reports worth keeping go in
`bench/published/`.

Other commands:

- `ptv-bench run --recorder playerstv` runs a single recorder once.
- `ptv-bench idle --recorder playerstv` measures a recorder with no game
  running.
- `ptv-bench analyze <clip> --log <frames.csv>` checks a clip.
- `ptv-bench render out.png --index 120` draws one test frame.

## Microbenchmarks

`cargo bench -p playerstv-core -p playerstv-bench --benches` times the Rust work
around a recording (building the ffmpeg command, scanning the buffer,
reading clip metadata) and the test game's drawing. CI runs these on every
push and keeps the numbers as an artifact, but doesn't fail on them: shared
runners are too noisy.

## Investigating a regression

When a metric gets worse, profile instead of guessing:

```powershell
wpr -start CPU -start GPU -filemode   # in an elevated prompt
target\release\ptv-bench.exe run --recorder playerstv --seconds 60 --yes
wpr -stop playerstv.etl
```

Open `playerstv.etl` in Windows Performance Analyzer. CPU Usage (Sampled)
shows where playerstv and ffmpeg spend their time, and GPU Utilization shows
the capture and encode work.
