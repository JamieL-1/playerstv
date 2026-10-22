// Mirrors of the Rust types the backend sends (serde JSON). Keep in sync with
// crates/core/src/config.rs, crates/core/src/profile.rs and
// crates/common/src/{status,control}.rs.

export type Resolution = "native" | "2x" | "720p" | "1080p" | "1440p" | "2160p";
export type Codec = "h264" | "hevc" | "av1";
export type Quality = "low" | "medium" | "high" | "very_high" | "max";
export type AudioSourceKind = "game" | "desktop";
export type BaseTheme = "dark" | "ash" | "onyx" | "light";

export interface VideoConfig {
  resolution: Resolution;
  fps: number;
  codec: Codec;
  quality: Quality;
  bitrate_cap_mbps: number | null;
}

export interface AudioConfig {
  game_audio: boolean;
  source: AudioSourceKind;
  mic: boolean;
  mic_device: string | null;
  game_volume: number;
  mic_volume: number;
  mixed_track: boolean;
  bitrate_kbps: number;
}

/** A per-game bitrate cap: automatic, or a fixed peak in Mbps. */
export type BitrateCap = "auto" | number;

/** Per-game replacements; a missing key follows the global setting. */
export interface VideoOverrides {
  resolution?: Resolution;
  fps?: number;
  codec?: Codec;
  quality?: Quality;
  bitrate_cap?: BitrateCap;
}

export interface AudioOverrides {
  game_audio?: boolean;
  source?: AudioSourceKind;
  mic?: boolean;
  game_volume?: number;
  mic_volume?: number;
  mixed_track?: boolean;
}

export interface GameOverrides {
  record_full_session?: boolean;
  buffer_seconds?: number;
  video?: VideoOverrides;
  audio?: AudioOverrides;
}

export interface GameProfile {
  id: string;
  name: string;
  exe_names: string[];
  window_title_regex: string;
  enabled: boolean;
  /** Absent when the game uses the global settings for everything. */
  overrides?: GameOverrides;
}

export interface Gradient {
  name: string;
  from: string;
  to: string;
}

export interface UiConfig {
  theme: BaseTheme;
  accent: string;
  gradient: Gradient | null;
}

/** What the window's close button does (`ask` shows a dialog each time). */
export type CloseAction = "ask" | "tray" | "quit";

export interface Config {
  buffer_seconds: number;
  clip_hotkey: string;
  session_hotkey: string;
  output_dir: string;
  record_full_session: boolean;
  start_with_windows: boolean;
  notifications: boolean;
  auto_update: boolean;
  close_action: CloseAction;
  storage_limit_gb: number | null;
  video: VideoConfig;
  audio: AudioConfig;
  games: GameProfile[];
  ui: UiConfig;
}

export type Status =
  | { state: "waiting" }
  | {
      state: "recording";
      game: string;
      since: number;
      session_since: number | null;
      video: string;
      buffer_seconds: number;
    }
  | { state: "failing"; game: string; error: string };

/** What the tray reports; `null` when the tray app isn't running. */
export interface Snapshot {
  status: Status;
  buffer_seconds: number;
  clip_hotkey: string;
  session_hotkey: string;
  output_dir: string;
  last_saved: string | null;
  /** Version of a downloaded update that a restart would install. */
  update_ready: string | null;
  update: UpdateState;
}

/** The tray's updater (crates/common/src/control.rs). Times are Unix seconds. */
export type UpdateState =
  | { state: "unavailable" }
  | { state: "idle" }
  | { state: "checking" }
  | { state: "up_to_date"; checked_at: number }
  | { state: "downloading"; version: string }
  | { state: "ready"; version: string }
  | { state: "failed"; error: string; at: number };

/** Ranges shared with crates/core/src/config.rs (sanitize). */
export const LIMITS = {
  bufferSeconds: [5, 600],
  fps: [30, 60, 90, 120],
  volume: [0, 400],
  audioBitrateKbps: [64, 320],
  bitrateCapMbps: [2, 400],
  storageLimitGb: [1, 20000],
} as const;

export const BUILTIN_GAME_IDS = ["runelite", "league-of-legends", "cs2"] as const;

/** crates/core/src/media.rs */
export interface VideoInfo {
  codec: string;
  width: number;
  height: number;
  fps: number;
  /** 4:2:0 H.264/HEVC/AV1; see also lib/library.ts::canPlay. */
  playable: boolean;
}

export interface MediaInfo {
  duration_s: number;
  video: VideoInfo | null;
  audio_tracks: string[];
}

/** crates/ui/src/library.rs */
export interface Recording {
  path: string;
  name: string;
  /** Game folder name (safeName of the game's name). */
  game: string;
  kind: "clip" | "session";
  size: number;
  /** Unix seconds. */
  modified: number;
  /** null while the file is still being written, or if it can't be read. */
  info: MediaInfo | null;
  thumb: string | null;
}

/** crates/ui/src/trim.rs */
export type ExportDone =
  | { state: "saved"; id: number; path: string }
  | { state: "failed"; id: number; error: string }
  | { state: "cancelled"; id: number };
