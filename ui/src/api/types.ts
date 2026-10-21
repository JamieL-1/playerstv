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

export interface GameProfile {
  id: string;
  name: string;
  exe_names: string[];
  window_title_regex: string;
  enabled: boolean;
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

export interface Config {
  buffer_seconds: number;
  clip_hotkey: string;
  session_hotkey: string;
  output_dir: string;
  record_full_session: boolean;
  start_with_windows: boolean;
  notifications: boolean;
  storage_limit_gb: number | null;
  video: VideoConfig;
  audio: AudioConfig;
  games: GameProfile[];
  ui: UiConfig;
}

export type Status =
  | { state: "waiting" }
  | { state: "recording"; game: string; since: number; session_since: number | null; video: string }
  | { state: "failing"; game: string; error: string };

/** What the tray reports; `null` when the tray app isn't running. */
export interface Snapshot {
  status: Status;
  buffer_seconds: number;
  clip_hotkey: string;
  session_hotkey: string;
  output_dir: string;
  last_saved: string | null;
}

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
