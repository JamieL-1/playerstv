# playerstv

A lightweight, plays.tv-style replay recorder for Windows. It sits in the
tray, notices when your game opens, keeps a rolling replay buffer, and saves
the last N seconds when you press a hotkey. Built in: **Old School RuneScape
(RuneLite)**, **League of Legends** (matches only, not the lobby) and
**Counter-Strike 2** — each can be switched on/off from the tray's
*Games to record* menu or the window, and you can add custom games.

- **Instant replay**: `Alt+F10` saves the last 30 s (configurable, 5–600 s).
- **Full sessions**: `Alt+F9` starts/stops recording the whole session, or
  enable it permanently in settings. Sessions in progress survive a crash.
- **GPU only**: Windows Graphics Capture → NVIDIA NVENC (H.264/HEVC/AV1),
  frames never touch the CPU. Up to 4K and 120 fps.
- **Audio**: either only the game's own process (no Discord/Spotify) or all
  desktop audio, plus your mic. Track 1 is a *Mixed* track (adjustable game/mic
  levels) so every player and YouTube hears everything; the separate
  *Game*/*Desktop* and *Mic* tracks follow for editing.
- **Always know what's recording**: the tray dot is grey (idle), green
  (replay buffer) or red (full session). Left-click it for the playerstv window,
  which shows the live session timer with Save clip / Stop & save session
  buttons. The tray menu shows `● REC 12:34` too.
- **Library, player and trim editor**: every clip and session per game, played
  in the app, with frame-exact trimming re-encoded on the GPU.

Measured on an RTX 5080 recording RuneLite at 1900×1030@60: ~0.5 % total CPU,
~11 % NVENC, tray process 9 MB private memory. A 30 s buffer is ~40 MB on disk.

## The window

Starting `playerstv.exe` opens a Discord-style window; closing it leaves
playerstv recording from the tray, and left-clicking the tray icon (or *Open
playerstv*) brings it back. The window is the same exe running as a second
process (`playerstv.exe --window`) that only exists while the window is open,
so recording costs nothing extra the rest of the time. The first time you
close it, a notification says playerstv is still running. To exit
completely, use *Quit* in the tray menu or *Quit playerstv* at the bottom of
the window's settings; either closes the window and the tray together.

- **Library**: a server rail with one icon per game, *All recordings / Clips /
  Sessions* with counts, and a thumbnail grid with search and sorting. It
  updates live as clips are saved. Right-click a recording (or use its `⋯`
  menu) to play, trim, show it in Explorer, open it in your default player or
  delete it (to the Recycle Bin, after a confirmation).
- **Player**: `Space`/`K` play/pause, `J`/`L` ±10 s, `←`/`→` ±5 s, `,`/`.`
  one frame, `F` full screen, `M` mute, `Esc` back. It plays the first audio
  track (*Mixed*). Older 4:4:4 recordings, and HEVC where WebView2 can't decode
  it, get an *Open in default player* button instead.
- **Trim editor**: drag the in/out handles or press `I`/`O` at the playhead;
  playback loops inside the selection. *Save clip* cuts exactly there,
  re-encodes the video with NVENC in the source's codec at your quality
  setting, and copies every audio track with its name. The result is saved as
  a new clip next to the original, and counts toward the storage limit.
- **Settings and themes**: everything in `config.toml`, plus Discord's
  Dark/Ash/Onyx/Light themes, an accent colour and gradient themes.

## Storage limit

Optional (off by default): *Settings → Storage → Limit how much space
recordings can use*. When saved clips + sessions go over the limit, the
**oldest are permanently deleted** until they fit. It's checked after every
save, at startup, and when you change the limit.

- Only files playerstv saved count — `.mp4`s in `<output>/<Game>/` and
  `<output>/<Game>/Sessions/` named like `RuneLite_2026-10-21_14-03-22.mp4`.
  Your own files in the folder are never touched; move or rename a recording
  to keep it forever.
- The newest recording is never deleted, even if it alone is over the limit.
- Nothing is deleted while another save is still being written; a file that's
  open elsewhere (e.g. playing) is skipped and retried after the next save.
- The window shows current usage; a notification says what was removed.

## Video quality

Pick a **Quality** level (Low → Maximum); playerstv maps it to an NVENC
constant-quality value per codec and caps the bitrate automatically from the
output size and frame rate. The encoder uses spatial/temporal AQ, B-frame
references, and — when there's headroom (up to ~1440p120) — multipass and
lookahead; above that it switches to a lighter preset and, for HEVC/AV1,
splits frames across the GPU's NVENC engines so 4K120 keeps up.

OSRS is mostly tiny coloured pixel text, which normal 4:2:0 video blurs
(colour is stored at half resolution). For sharp text, use the
**2× pixel-perfect** resolution: every game pixel becomes a 2×2 block, so
4:2:0 colour lands on exactly one sample per original pixel. It plays
everywhere and YouTube serves it at 4K bitrates; files are ~2.5× bigger.

Measured against a lossless capture of RuneLite (SSIM over RGB, higher is
better): old cq 35 → 13.1 dB, tuned 4:2:0 → 16.5 dB.

Everything playerstv records is 4:2:0 H.264/HEVC/AV1 so the built-in player
can play it. (4:4:4 was removed for that reason.)

Choosing a fixed resolution larger than the game window upscales (bicubic):
no extra detail locally, though YouTube gives higher resolutions more bitrate.

## Download

Grab `playerstv-vX.Y.Z-windows-x64.zip` from the
[Releases](../../releases) page, unzip it anywhere and run `playerstv.exe`
FFmpeg is built into the exe and unpacked to
`%LOCALAPPDATA%\playerstv\ffmpeg` on first run; nothing else needs
installing (the WebView2 runtime the window uses ships with Windows 11 and
most Windows 10 PCs, and playerstv offers the download if it's missing). The app isn't
code-signed yet, so SmartScreen may ask: **More info → Run anyway**.

### Making a release

**Actions → Release → Run workflow**, pick a version bump (`patch`, `minor`,
`major`, or `none` to release the version already in `Cargo.toml`) and run.
`.github/workflows/release.yml` then bumps the version, tests, builds and
packages (`scripts/package.ps1`), and — only if all of that passes — pushes a
`Release vX.Y.Z` commit to `main`, creates the `vX.Y.Z` tag and publishes the
release with the zip and its SHA-256. Pull afterwards to get the bump commit.

Tick *dry run* to build without pushing or publishing; the zip is attached to
the run as an artifact.

## Requirements

- Windows 10 2004+ / Windows 11 (per-process audio loopback, WGC capture)
- NVIDIA GPU with NVENC
- Microsoft Edge WebView2 runtime for the window (preinstalled on Windows 11)
- FFmpeg **8.0 or newer** (needs the `gfxcapture` filter) — fetched by script
- To build: Rust (stable) and Node.js 24

## Build & run

```powershell
powershell -File scripts/fetch-ffmpeg.ps1   # puts ffmpeg.exe in vendor/ffmpeg
cd ui; npm ci; npm run build; cd ..         # the window's frontend (embedded in playerstv.exe)
cargo build --release
target\release\playerstv.exe                # tray + window
```

Dev builds use `vendor/ffmpeg/ffmpeg.exe` (or an `ffmpeg.exe` next to the
exe, which always wins). For distribution, build with
`cargo build --release --features bundle-ffmpeg` to embed ffmpeg in the exe,
then `scripts/package.ps1 -Version vX.Y.Z` zips it with a README for
testers (`scripts/README-tester.txt`) and the licence notices.

| Command | What it does |
|---|---|
| `playerstv` | Start the tray and open the window (if it's already running, just open the window) |
| `playerstv --background` | Start the tray only (used by *Start with Windows*) |
| `playerstv --window` | The window on its own; the tray starts it this way |
| `playerstv --smoke 15` | Headless check: wait for the game, record 15 s, save a clip, print its path |

## Files

| What | Where |
|---|---|
| Config | `%APPDATA%\playerstv\config.toml` (or `PLAYERSTV_CONFIG`) |
| Clips | `%USERPROFILE%\Videos\playerstv\<Game>\` |
| Sessions | `%USERPROFILE%\Videos\playerstv\<Game>\Sessions\` |
| Replay buffer | `%LOCALAPPDATA%\playerstv\buffer\` (deleted when the game closes) |
| Logs | `%LOCALAPPDATA%\playerstv\logs\` (`PLAYERSTV_LOG=debug` for more) |
| Library cache | `%LOCALAPPDATA%\playerstv\library.json` (metadata) and `thumbs\` |

The window writes `config.toml`; the tray app watches the file and
applies changes immediately (changing video/audio settings restarts the
encoder). Built-in games keep their detection rules in code (only the on/off
switch is stored), so detection fixes arrive with updates. A custom game is a
`[[games]]` entry with executable names and a window-title regex.

`PLAYERSTV_INSTANCE=<name>` runs a completely separate instance (own locks,
control pipe, buffer and logs) next to the real one — use it with
`PLAYERSTV_CONFIG` for testing. Live buffers also hold an exclusive
`in-use.lock`, so no other playerstv process will ever treat them as crash
leftovers.

## How it works

```
 RuneLite window ──WGC──► ffmpeg gfxcapture ──D3D11──► NVENC ──► 2 s .ts segments ──┐
 RuneLite audio ─WASAPI process loopback─► named pipe ─┤                           │
 Microphone ─────WASAPI capture──────────► named pipe ─┘                           │
                                                                                     ▼
                          hotkey ──► stream-copy concat of newest segments ──► clip.mp4

 playerstv.exe (tray) ◄──control pipe: status / clip / session──► playerstv.exe --window
        ▲                                                           │  Tauri + WebView2
        └────────────── config.toml (both watch it) ────────────────┤  React frontend (ui/)
                                                                    ▼
                         output folder ──► library (metadata, ffmpeg thumbnails)
                                       ──► player (asset protocol) ──► trim ──► NVENC ──► new clip
```

- `crates/core` — config, game profiles, ffmpeg argument builders (recording,
  clip export, trim), segment ring-buffer maths, clip naming, storage limit,
  and a small MP4 reader for library metadata. Pure and unit-tested
  (`cargo test`).
- `crates/capture` — window detection (Win32) and audio feeders (WASAPI →
  named pipes). Feeders run on a wall clock and pad silence, so a silent game
  never stalls the audio track.
- `crates/common` — paths, the control pipe and process helpers shared by the
  tray and the window.
- `crates/app` (`playerstv-tray`) — the tray as a library: tray, hotkeys,
  supervisor state machine.
- `crates/ui` (`playerstv`) — builds `playerstv.exe`: runs the tray, or with
  `--window` the window's backend (Tauri 2): status, settings, library,
  thumbnails, trim exports.
- `ui/` — the window's frontend: React, TypeScript, shadcn/ui on Tailwind,
  built with Vite+ (`vp`).

Design notes:

- The buffer is on disk, not in RAM, so memory stays flat at any length.
  Saving is a stream copy (~0.2–1 s, no re-encode). When you press the
  hotkey we wait for the in-progress segment to close, so the clip includes
  the moment you pressed it (plus up to 2 s after).
- Every encoder start begins a new "epoch". A clip never mixes footage from
  two encoders, and a full session is split into a new file if the encoder
  restarts (settings change or encoder failure), because concatenating
  different resolutions/codecs produces broken files.
- ffmpeg runs inside a kill-on-close Job Object, so it can't outlive the app.

## Development

CI (`.github/workflows/ci.yml`) runs on every push to `main` and every pull
request; run the same checks locally before pushing:

```powershell
cd ui; npm run check; npm test; npm run build; cd ..   # first: crates/ui embeds ui/dist
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

It also runs `cargo audit` against the RustSec advisory database, and
Dependabot opens weekly PRs for crate, npm and Actions updates. Formatting uses
`rustfmt.toml` (120 columns); `record_args` opts out so each ffmpeg flag
stays next to its value.

**Window development with live reload**: run `npm run dev` in `ui/`, then
`cargo run -p playerstv --no-default-features -- --window` (without the default
`custom-protocol` feature the window loads the Vite dev server instead of the
embedded files). Any frontend change needs `npm run build` before a normal
`cargo build` picks it up.

**UI test mode** (debug builds only) opens the window off-screen and
unfocused, so tests never pop up over a game:

| Variable | Effect |
|---|---|
| `PLAYERSTV_UI_TEST=1` | Enable test mode |
| `PLAYERSTV_UI_ROUTE=/settings/video` | Start on a page (e.g. `/game/runelite?kind=clips`, `/watch?path=<url-encoded>`) |
| `PLAYERSTV_UI_EVAL=<js>` | Run a script half a second after load, to drive the UI |
| `PLAYERSTV_UI_SCREENSHOT=out.png` | Save a screenshot (via WebView2) and exit |
| `PLAYERSTV_UI_SETTLE_MS=2500` | How long to wait before the screenshot |

Set them for `playerstv.exe --window`, or for `playerstv.exe` to test the tray
and window together (the window inherits them). Combine it with
`PLAYERSTV_INSTANCE=test` and a `PLAYERSTV_CONFIG` whose games are all
disabled. From Git Bash, set `MSYS_NO_PATHCONV=1` so routes aren't
rewritten into paths.

## Diagnostics

```powershell
cargo run -p playerstv-capture --example session_peaks          # per-app peak + mixer volume
cargo run -p playerstv-capture --example loopback_probe -- <pid> # what loopback hears from a process
```

Process loopback records what the app *actually* outputs after Windows
processing, so a quiet Game track usually means the game itself is quiet or
attenuated (in-game volume, Windows communications ducking during calls).

## Not done yet / known gaps

- YouTube upload (planned: OAuth loopback flow + resumable upload API).
- The in-app player plays only the first audio track (*Mixed*); trimmed clips
  keep all of them.
- Behaviour while the game window is minimised hasn't been tested.
- Windows logoff/shutdown doesn't trigger a graceful save; an in-progress
  session is recovered on next launch instead.
- NVIDIA only (AMD AMF / Intel QSV would be a small change in
  `ffmpeg_args.rs`).
- `Alt+F10` is also NVIDIA App's default Instant Replay hotkey; if both are
  enabled they'll both fire. Change one of them in settings.
