import { describe, expect, it } from "vitest";
import type { Config } from "@/api/types";
import { isDirty, mergeExternalChange } from "./settingsDraft";

const base: Config = {
  buffer_seconds: 30,
  clip_hotkey: "Alt+F10",
  session_hotkey: "Alt+F9",
  output_dir: "C:\\Videos\\playerstv",
  record_full_session: false,
  start_with_windows: false,
  notifications: true,
  storage_limit_gb: null,
  video: { resolution: "native", fps: 60, codec: "h264", quality: "high", bitrate_cap_mbps: null },
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
    { id: "cs2", name: "Counter-Strike 2", exe_names: ["cs2.exe"], window_title_regex: "^Counter", enabled: true },
  ],
  ui: { theme: "dark", accent: "#5865f2", gradient: null },
};

const withGame = (c: Config, id: string, enabled: boolean): Config => ({
  ...c,
  games: c.games.map((g) => (g.id === id ? { ...g, enabled } : g)),
});

describe("mergeExternalChange", () => {
  it("takes the new config when there are no unsaved edits", () => {
    const incoming = { ...base, buffer_seconds: 45 };
    expect(mergeExternalChange(base, base, incoming)).toEqual(incoming);
  });

  it("keeps unsaved edits but takes instant-saved fields", () => {
    const draft = { ...base, buffer_seconds: 90 }; // unsaved edit
    const incoming = { ...withGame(base, "cs2", false), ui: { ...base.ui, accent: "#eb459e" } };
    const merged = mergeExternalChange(draft, base, incoming);
    expect(merged.buffer_seconds).toBe(90);
    expect(merged.games.find((g) => g.id === "cs2")!.enabled).toBe(false);
    expect(merged.ui.accent).toBe("#eb459e");
  });
});

describe("isDirty", () => {
  it("compares by value", () => {
    expect(isDirty(base, structuredClone(base))).toBe(false);
    expect(isDirty({ ...base, buffer_seconds: 31 }, base)).toBe(true);
    expect(isDirty(undefined, base)).toBe(false);
  });
});
