import { describe, expect, it } from "vitest";
import type { Recording } from "@/api/types";
import {
  canPlay,
  countKinds,
  defaultClipName,
  displayName,
  filterRecordings,
  formatWhen,
  gameName,
  nameError,
  parseKind,
  sortRecordings,
} from "./library";

function rec(name: string, over: Partial<Recording> = {}): Recording {
  return {
    path: `C:\\v\\${name}`,
    name,
    custom_name: null,
    game: "RuneLite",
    kind: name.includes("_session_") ? "session" : "clip",
    size: 100,
    modified: 1000,
    info: {
      duration_s: 30,
      video: { codec: "h264", width: 1920, height: 1080, fps: 60, playable: true },
      audio_tracks: [],
    },
    thumb: null,
    ...over,
  };
}

const recs = [
  rec("RuneLite_2026-10-21_14-03-22.mp4", { modified: 300, size: 50 }),
  rec("RuneLite_session_2026-10-20_10-00-00.mp4", { modified: 100, size: 900 }),
  rec("Counter-Strike_2_2026-10-21_20-00-00.mp4", { game: "Counter-Strike_2", modified: 200, size: 10 }),
];

describe("filterRecordings", () => {
  it("filters by game folder and kind", () => {
    const names = (f: Parameters<typeof filterRecordings>[1]) => filterRecordings(recs, f).map((r) => r.name);
    expect(names({ folder: null, kind: null, search: "" })).toHaveLength(3);
    expect(names({ folder: "RuneLite", kind: null, search: "" })).toHaveLength(2);
    expect(names({ folder: "RuneLite", kind: "sessions", search: "" })).toEqual([
      "RuneLite_session_2026-10-20_10-00-00.mp4",
    ]);
    expect(names({ folder: null, kind: "clips", search: "" })).toHaveLength(2);
  });

  it("searches every word, case-insensitively", () => {
    expect(filterRecordings(recs, { folder: null, kind: null, search: "rune 10-21" })).toHaveLength(1);
    expect(filterRecordings(recs, { folder: null, kind: null, search: "COUNTER" })).toHaveLength(1);
    expect(filterRecordings(recs, { folder: null, kind: null, search: "nothing" })).toHaveLength(0);
  });
});

describe("sortRecordings", () => {
  const order = (o: Parameters<typeof sortRecordings>[1]) => sortRecordings(recs, o).map((r) => r.modified);
  it("sorts by date, size and length", () => {
    expect(order("newest")).toEqual([300, 200, 100]);
    expect(order("oldest")).toEqual([100, 200, 300]);
    expect(order("largest")).toEqual([100, 300, 200]);
  });

  it("puts unknown durations last and breaks ties newest first", () => {
    const withLong = [...recs, rec("RuneLite_2026-10-22_00-00-00.mp4", { modified: 50, info: null })];
    withLong[0] = { ...withLong[0]!, info: { ...withLong[0]!.info!, duration_s: 99 } };
    expect(sortRecordings(withLong, "longest").map((r) => r.modified)).toEqual([300, 200, 100, 50]);
  });

  it("doesn't mutate its input", () => {
    const copy = [...recs];
    sortRecordings(recs, "oldest");
    expect(recs).toEqual(copy);
  });
});

describe("helpers", () => {
  it("counts kinds per game", () => {
    expect(countKinds(recs, null)).toEqual({ all: 3, clips: 2, sessions: 1 });
    expect(countKinds(recs, "RuneLite")).toEqual({ all: 2, clips: 1, sessions: 1 });
    expect(countKinds(recs, "Minecraft")).toEqual({ all: 0, clips: 0, sessions: 0 });
  });

  it("parses the kind query parameter", () => {
    expect(parseKind("clips")).toBe("clips");
    expect(parseKind("sessions")).toBe("sessions");
    expect(parseKind("junk")).toBeNull();
    expect(parseKind(null)).toBeNull();
  });

  it("names games from the config, or from the folder", () => {
    const games = [{ id: "cs2", name: "Counter-Strike 2", exe_names: [], window_title_regex: "", enabled: true }];
    expect(gameName(recs[2]!, games)).toBe("Counter-Strike 2");
    expect(gameName(recs[0]!, games)).toBe("RuneLite");
    expect(gameName(rec("x", { game: "Old_Game" }), games)).toBe("Old Game");
  });

  it("knows what the player can show", () => {
    expect(canPlay(recs[0]!, false)).toBe(true);
    const hevc = rec("a", { info: { ...recs[0]!.info!, video: { ...recs[0]!.info!.video!, codec: "hevc" } } });
    expect(canPlay(hevc, true)).toBe(true);
    expect(canPlay(hevc, false)).toBe(false);
    const legacy = rec("b", { info: { ...recs[0]!.info!, video: { ...recs[0]!.info!.video!, playable: false } } });
    expect(canPlay(legacy, true)).toBe(false);
    expect(canPlay(rec("c", { info: null }), true)).toBe(false);
  });

  it("formats dates like Discord", () => {
    const now = new Date(2026, 9, 22, 12, 0, 0);
    const at = (d: Date) => Math.floor(d.getTime() / 1000);
    expect(formatWhen(at(new Date(2026, 9, 22, 9, 5)), now)).toMatch(/^Today at /);
    expect(formatWhen(at(new Date(2026, 9, 21, 23, 59)), now)).toMatch(/^Yesterday at /);
    expect(formatWhen(at(new Date(2026, 9, 1, 8, 0)), now)).not.toMatch(/day at/);
  });
});

describe("names", () => {
  it("shows the custom name, else the file name", () => {
    expect(displayName(rec("RuneLite_2026-10-21_14-03-22.mp4"))).toBe("RuneLite_2026-10-21_14-03-22");
    expect(displayName(rec("Big fight_2026-10-21_14-03-22.mp4", { custom_name: "Big fight" }))).toBe("Big fight");
  });

  it("defaults to the game and the time, like the backend", () => {
    expect(defaultClipName("RuneLite", new Date(2026, 9, 3, 4, 5, 6))).toBe("RuneLite_2026-10-03_04-05-06");
  });

  it("rejects names that can't be file names", () => {
    expect(nameError("Big fight")).toBeNull();
    expect(nameError("  Big fight.mp4 ")).toBeNull();
    expect(nameError("   ")).toBe("Enter a name");
    expect(nameError(".mp4")).toBe("Enter a name");
    expect(nameError("a/b")).toBe("Names can't contain /");
    expect(nameError('say "gg"')).toBe("Names can't contain \"");
    expect(nameError("tab\there")).toMatch(/control/);
    expect(nameError("x".repeat(100))).toBeNull();
    expect(nameError("x".repeat(101))).toMatch(/at most 100/);
  });
});
