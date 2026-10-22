# playerstv

A replay recorder for Windows. It runs in the tray, starts recording when a
supported game opens, and saves the last 30 seconds when you press a hotkey.

Supported games are Old School RuneScape (RuneLite), League of Legends
(matches only) and Counter-Strike 2. You can add your own.

- `Alt+F10` saves the last 30 s. The length is configurable from 5 to 600 s.
- `Alt+F9` starts or stops recording the whole session. A session in progress
  survives a crash.
- Frames stay on the GPU from capture to the NVENC encoder (H.264, HEVC or
  AV1), up to 4K at 120 fps.
- Audio is the game alone or all desktop audio, plus your mic. Track 1 mixes
  them, because most players only play the first track.
- The tray dot is grey when idle, green with the replay buffer on, and red
  while recording a session.
- The window has a library, a player and a frame-exact trim editor.

On an RTX 5080 recording RuneLite at 1900×1030, 60 fps: about 0.5% CPU, 11%
NVENC and 9 MB of memory. A 30 s buffer takes about 40 MB of disk.

## Download

Get `playerstv-vX.Y.Z-windows-x64.zip` from [Releases](../../releases), unzip
it and run `playerstv.exe`. The app isn't code-signed yet, so SmartScreen may
ask you to click **More info**, then **Run anyway**.

You need Windows 10 2004 or later, an NVIDIA GPU, and the WebView2 runtime.
Windows 11 includes WebView2, and playerstv offers the download if it's
missing.

## Using it

Closing the window quits playerstv. To record without the window, turn on
*Start with Windows* or run `playerstv.exe --background`.

In the player, `Space` or `K` plays and pauses, `J`/`L` skip 10 s, the arrow
keys skip 5 s, `,`/`.` step one frame and `F` toggles full screen. In the trim
editor, `I` and `O` set the in and out points.

**Storage limit.** If you set one in *Settings → Storage*, playerstv deletes
the oldest recordings when the total goes over it. It only deletes files it
named itself and never deletes the newest one.

**Sharp text in OSRS.** Use the *2× pixel-perfect* resolution. Normal video
blurs small coloured text, and doubling every pixel avoids that. Files are
about 2.5× bigger.

**Updates.** playerstv checks for a new release at startup and every 6 hours,
then downloads it in the background. It installs the update when nothing is
recording, when you click **Restart** on the update banner, or when you quit.
It rejects a download that doesn't match the SHA-256 and minisign signature in
`latest.json`. Turn it off with *Settings → Update automatically*.

## Files

| What | Where |
|---|---|
| Config | `%APPDATA%\playerstv\config.toml`, or the path in `PLAYERSTV_CONFIG` |
| Recordings | `%USERPROFILE%\Videos\playerstv\<Game>\`, sessions in `Sessions\` |
| Replay buffer | `%LOCALAPPDATA%\playerstv\buffer\`, deleted when the game closes |
| Logs | `%LOCALAPPDATA%\playerstv\logs\`. Set `PLAYERSTV_LOG=debug` for more. |

The tray watches `config.toml` and applies changes immediately. A custom game
is a `[[games]]` entry with executable names and a window-title regex.

## Building

You need Rust (stable) and Node.js 24.

```powershell
powershell -File scripts/fetch-ffmpeg.ps1   # FFmpeg 8.0+ into vendor/ffmpeg
cd ui; npm ci; npm run build; cd ..         # frontend, embedded in the exe
cargo build --release
target\release\playerstv.exe
```

For a release build, add `--features bundle-ffmpeg` to embed FFmpeg, then run
`scripts/package.ps1 -Version vX.Y.Z` to make the zip. Only these builds
update themselves.

Run the CI checks before pushing:

```powershell
cd ui; npm run check; npm test; npm run build; cd ..
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

For live reload, run `npm run dev` in `ui/`, then
`cargo run -p playerstv --no-default-features -- --window`.

### Code layout

- `crates/core`: config, game profiles, ffmpeg arguments, the segment buffer
  and the storage limit. Pure and unit-tested.
- `crates/capture`: window detection and WASAPI audio capture.
- `crates/common`: paths, the control pipe and process helpers.
- `crates/app`: the tray, hotkeys, the recording supervisor and the updater.
- `crates/ui`: builds `playerstv.exe` and the window's Tauri backend.
- `ui/`: the window's React frontend.

FFmpeg writes 2-second segments to disk. Saving a clip joins the newest
segments without re-encoding, which takes up to a second.

### Testing

`PLAYERSTV_INSTANCE=<name>` runs a separate instance with its own locks, pipe,
buffer and logs. Pair it with `PLAYERSTV_CONFIG` so tests don't touch your real
setup.

Debug builds also have a UI test mode that opens the window off-screen:

| Variable | Effect |
|---|---|
| `PLAYERSTV_UI_TEST=1` | Turn on test mode |
| `PLAYERSTV_UI_ROUTE=/settings/video` | Open this page first |
| `PLAYERSTV_UI_EVAL=<js>` | Run a script 0.5 s after load |
| `PLAYERSTV_UI_SCREENSHOT=out.png` | Save a screenshot and exit |
| `PLAYERSTV_UI_SETTLE_MS=2500` | Wait this long before the screenshot |

From Git Bash, set `MSYS_NO_PATHCONV=1` so routes stay as typed.

### Releasing

Run **Actions → Release** and pick a version bump. The workflow tests, builds,
packages and signs the release. If every step passes, it pushes the version
commit and tag and publishes the release. Pull afterwards to get the commit.
Tick *dry run* to build without publishing.

Signing needs the `MINISIGN_KEY` secret, which must match `PUBLIC_KEY` in
`crates/app/src/updater.rs`. If you lose the key, generate a new pair with
`minisign -G -W`, update `PUBLIC_KEY`, and send that build to testers by hand.

## Known gaps

- NVIDIA only. AMD and Intel encoders would need changes in `ffmpeg_args.rs`.
- The player plays only the first audio track. Trimmed clips keep every track.
- Logging off or shutting down Windows doesn't save the session. playerstv
  recovers it on the next start.
- `Alt+F10` is also NVIDIA App's Instant Replay hotkey. If both are on, change
  one of them.
