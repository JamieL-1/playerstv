import { describe, expect, it } from "vitest";
import type { Config } from "@/api/types";
import { isDirty, mergeExternalChange, restoreDefaults } from "./settingsDraft";

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

  it("takes a close action saved from the close dialog, unless it was edited here", () => {
    const draft = { ...base, buffer_seconds: 90 };
    const incoming: Config = { ...base, close_action: "tray" };
    expect(mergeExternalChange(draft, base, incoming).close_action).toBe("tray");
    const edited: Config = { ...draft, close_action: "quit" };
    expect(mergeExternalChange(edited, base, incoming).close_action).toBe("quit");
  });
});

describe("isDirty", () => {
  it("compares by value", () => {
    expect(isDirty(base, structuredClone(base))).toBe(false);
    expect(isDirty({ ...base, buffer_seconds: 31 }, base)).toBe(true);
    expect(isDirty(undefined, base)).toBe(false);
  });
});

describe("restoreDefaults", () => {
  it("resets settings but keeps games, output folder and appearance", () => {
    const current: Config = {
      ...withGame(base, "cs2", false),
      buffer_seconds: 120,
      record_full_session: true,
      output_dir: "D:/Clips",
      ui: { ...base.ui, accent: "#eb459e" },
    };
    current.games[0] = { ...current.games[0]!, overrides: { video: { fps: 120 } } };
    const defaults: Config = { ...base, games: [], output_dir: "C:/default/playerstv" };
    const restored = restoreDefaults(current, defaults);
    expect(restored.buffer_seconds).toBe(30);
    expect(restored.record_full_session).toBe(false);
    expect(restored.output_dir).toBe("D:/Clips");
    expect(restored.ui.accent).toBe("#eb459e");
    expect(restored.games.map((g) => g.id)).toEqual(["runelite", "cs2"]);
    expect(restored.games[1]!.enabled).toBe(false);
    expect("overrides" in restored.games[0]!).toBe(false);
  });

  it("keeps the app settings", () => {
    const current: Config = {
      ...base,
      start_with_windows: true,
      notifications: false,
      auto_update: false,
      close_action: "tray",
    };
    const restored = restoreDefaults(current, base);
    expect(restored.start_with_windows).toBe(true);
    expect(restored.notifications).toBe(false);
    expect(restored.auto_update).toBe(false);
    expect(restored.close_action).toBe("tray");
  });
});
