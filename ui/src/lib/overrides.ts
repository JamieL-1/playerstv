import type { AudioOverrides, Config, GameOverrides, GameProfile, VideoOverrides } from "@/api/types";

// Key order of the Rust structs (crates/core/src/config.rs), so a pruned
// draft compares equal to what the backend sends back.
const VIDEO_KEYS = ["resolution", "fps", "codec", "quality", "bitrate_cap"] as const;
const AUDIO_KEYS = ["game_audio", "source", "mic", "game_volume", "mic_volume", "mixed_track"] as const;

function pick<T extends object>(o: T | undefined, keys: readonly (keyof T)[]): T | undefined {
  if (!o) return undefined;
  const out = {} as T;
  for (const k of keys) if (o[k] !== undefined && o[k] !== null) out[k] = o[k];
  return Object.keys(out).length ? out : undefined;
}

/**
 * Drops unset fields and empty groups, in the backend's key order. Returns
 * undefined when nothing is overridden, matching a config.toml without
 * an `overrides` table (so setting then unsetting a field isn't "unsaved").
 */
export function pruneOverrides(o: GameOverrides | undefined): GameOverrides | undefined {
  if (!o) return undefined;
  const out: GameOverrides = {};
  if (o.record_full_session !== undefined && o.record_full_session !== null)
    out.record_full_session = o.record_full_session;
  if (o.buffer_seconds !== undefined && o.buffer_seconds !== null) out.buffer_seconds = o.buffer_seconds;
  const video = pick<VideoOverrides>(o.video, VIDEO_KEYS);
  if (video) out.video = video;
  const audio = pick<AudioOverrides>(o.audio, AUDIO_KEYS);
  if (audio) out.audio = audio;
  return Object.keys(out).length ? out : undefined;
}

/** Applies `fn` to one game's overrides, keeping the result pruned. */
export function updateOverrides(config: Config, gameId: string, fn: (o: GameOverrides) => GameOverrides): Config {
  return {
    ...config,
    games: config.games.map((g) => {
      if (g.id !== gameId) return g;
      const { overrides, ...rest } = g;
      const next = pruneOverrides(fn(overrides ?? {}));
      return next ? { ...rest, overrides: next } : rest;
    }),
  };
}

/** The settings a game records with, like Config::for_game in Rust. */
export function effectiveSettings(config: Config, game: GameProfile) {
  const o = game.overrides ?? {};
  const { bitrate_cap, ...video } = o.video ?? {};
  return {
    record_full_session: o.record_full_session ?? config.record_full_session,
    buffer_seconds: o.buffer_seconds ?? config.buffer_seconds,
    video: {
      ...config.video,
      ...video,
      bitrate_cap_mbps:
        bitrate_cap === undefined ? config.video.bitrate_cap_mbps : bitrate_cap === "auto" ? null : bitrate_cap,
    },
    audio: { ...config.audio, ...o.audio },
  };
}

/** "1 min", "90 s". */
export function formatBuffer(seconds: number): string {
  return seconds % 60 === 0 ? `${seconds / 60} min` : `${seconds} s`;
}

const CODEC_SHORT = { h264: "H.264", hevc: "HEVC", av1: "AV1" } as const;
const QUALITY_SHORT = { low: "Low", medium: "Medium", high: "High", very_high: "Very high", max: "Max" } as const;

/** One line for the game list, e.g. "Full session · 120 fps · Desktop audio"; "" for none. */
export function overrideSummary(o: GameOverrides | undefined): string {
  if (!o) return "";
  const parts: string[] = [];
  if (o.record_full_session !== undefined) parts.push(o.record_full_session ? "Full session" : "Clips only");
  if (o.buffer_seconds !== undefined) parts.push(`${formatBuffer(o.buffer_seconds)} buffer`);
  const v = o.video ?? {};
  if (v.resolution) parts.push(v.resolution === "native" ? "Native size" : v.resolution);
  if (v.fps !== undefined) parts.push(`${v.fps} fps`);
  if (v.codec) parts.push(CODEC_SHORT[v.codec]);
  if (v.quality) parts.push(`${QUALITY_SHORT[v.quality]} quality`);
  if (v.bitrate_cap !== undefined) parts.push(v.bitrate_cap === "auto" ? "Auto bitrate" : `${v.bitrate_cap} Mbps cap`);
  const a = o.audio ?? {};
  if (a.game_audio === false) parts.push("No computer audio");
  else if (a.source) parts.push(a.source === "game" ? "Game audio" : "Desktop audio");
  else if (a.game_audio) parts.push("Computer audio on");
  if (a.mic !== undefined) parts.push(a.mic ? "Mic on" : "No mic");
  if (a.mixed_track !== undefined) parts.push(a.mixed_track ? "Mixed track" : "No mixed track");
  if (a.game_volume !== undefined || a.mic_volume !== undefined) parts.push("Custom mix volumes");
  return parts.join(" · ");
}
