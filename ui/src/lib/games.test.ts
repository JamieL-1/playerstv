import { describe, expect, it } from "vitest";
import { gameAvatar, initials, safeName } from "./games";

describe("game avatars", () => {
  it("uses fixed avatars for built-in games", () => {
    expect(gameAvatar({ id: "cs2", name: "Counter-Strike 2" }).initials).toBe("CS2");
    expect(gameAvatar({ id: "runelite", name: "RuneLite" }).initials).toBe("RL");
  });

  it("derives initials for custom games, stably", () => {
    expect(initials("Minecraft")).toBe("MI");
    expect(initials("Old School: RuneScape")).toBe("OSR");
    expect(initials("  ")).toBe("?");
    const a = gameAvatar({ id: "custom-minecraft", name: "Minecraft" });
    expect(gameAvatar({ id: "custom-minecraft", name: "Minecraft" })).toEqual(a);
  });
});

describe("safeName", () => {
  it("matches crates/core profile::safe_name", () => {
    expect(safeName("RuneLite")).toBe("RuneLite");
    expect(safeName("Old School: RS")).toBe("Old_School__RS");
    expect(safeName("")).toBe("Game");
  });
});
