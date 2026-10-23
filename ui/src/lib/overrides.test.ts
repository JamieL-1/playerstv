import { describe, expect, it } from "vitest";
import type { Config } from "@/api/types";
import { effectiveSettings, overrideSummary, pruneOverrides, updateOverrides } from "./overrides";
import { isDirty } from "./settingsDraft";

const base: Config = {
  buffer_seconds: 30,
  clip_hotkey: "Alt+F10",
  session_hotkey: "Alt+F9",
  output_dir: "C:\\Videos\\playerstv",
  record_full_session: false,
  start_with_windows: false,
  notifications: true,
  auto_update: true,
  close_action: "ask",
  storage_limit_gb: null,
  keep_named_recordings: false,
  video: { resolution: "native", fps: 60, codec: "h264", quality: "high", bitrate_cap_mbps: 40 },
  audio: {
    game_audio: true,
    source: "game",
    mic: true,
    mic_device: null,
    game_volume: 100,
    mic_volume: 100,
    mixed_track: true,
    bitrate_kbps: 160,
  },
  games: [
    { id: "runelite", name: "RuneLite", exe_names: ["RuneLite.exe"], window_title_regex: "^RuneLite", enabled: true },
    {
      id: "league-of-legends",
      name: "League of Legends",
      exe_names: ["League of Legends.exe"],
      window_title_regex: "^League",
      enabled: true,
      overrides: { record_full_session: true, video: { fps: 120 } },
    },
  ],
  ui: { theme: "dark", accent: "#5865f2", gradient: null },
};

describe("overrides", () => {
  it("prunes unset fields and empty groups", () => {
    expect(pruneOverrides({ video: { fps: undefined }, audio: {} })).toBeUndefined();
    expect(pruneOverrides({ buffer_seconds: 60, video: { codec: undefined, fps: 90 } })).toEqual({
      buffer_seconds: 60,
      video: { fps: 90 },
    });
  });

  it("setting then unsetting a field isn't an unsaved change", () => {
    const set = updateOverrides(base, "runelite", (o) => ({ ...o, video: { ...o.video, fps: 30 } }));
    expect(isDirty(set, base)).toBe(true);
    const unset = updateOverrides(set, "runelite", (o) => ({ ...o, video: { ...o.video, fps: undefined } }));
    expect(isDirty(unset, base)).toBe(false);
    expect("overrides" in unset.games[0]!).toBe(false);
  });

  it("keeps the backend's key order", () => {
    const edited = updateOverrides(base, "league-of-legends", (o) => ({ video: o.video, ...o }));
    expect(isDirty(edited, base)).toBe(false);
  });

  it("layers a game's overrides over the globals", () => {
    const league = effectiveSettings(base, base.games[1]!);
    expect(league.record_full_session).toBe(true);
    expect(league.video.fps).toBe(120);
    expect(league.video.codec).toBe("h264");
    expect(league.buffer_seconds).toBe(30);
    expect(league.video.bitrate_cap_mbps).toBe(40);
    const auto = effectiveSettings(base, { ...base.games[0]!, overrides: { video: { bitrate_cap: "auto" } } });
    expect(auto.video.bitrate_cap_mbps).toBeNull();
  });

  it("summarises what's customised", () => {
    expect(overrideSummary(undefined)).toBe("");
    expect(overrideSummary({ record_full_session: true, video: { fps: 120 }, audio: { source: "desktop" } })).toBe(
      "Full session · 120 fps · Desktop audio",
    );
    expect(overrideSummary({ buffer_seconds: 120, audio: { game_audio: false, mic: false } })).toBe(
      "2 min buffer · No computer audio · No mic",
    );
  });
});
