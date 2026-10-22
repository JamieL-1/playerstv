import { describe, expect, it } from "vitest";
import type { UiConfig } from "@/api/types";
import { BASE_THEMES, GRADIENT_PRESETS, buildTheme, gradientMode, hexToRgb, luminance, readableOn } from "./themes";

const ui = (over: Partial<UiConfig> = {}): UiConfig => ({ theme: "dark", accent: "#5865f2", gradient: null, ...over });

describe("colour maths", () => {
  it("parses hex and computes luminance", () => {
    expect(hexToRgb("#5865f2")).toEqual([88, 101, 242]);
    expect(luminance("#000000")).toBe(0);
    expect(luminance("#ffffff")).toBeCloseTo(1);
    expect(() => hexToRgb("red")).toThrow();
  });

  it("picks readable text on accents", () => {
    expect(readableOn("#5865f2")).toBe("#ffffff"); // blurple
    expect(readableOn("#f0b232")).toBe("#111214"); // yellow
  });
});

describe("buildTheme", () => {
  it("uses Discord's dark greys by default", () => {
    const t = buildTheme(ui());
    expect(t.mode).toBe("dark");
    expect(t.vars["--rail"]).toBe("#1e1f22");
    expect(t.vars["--sidebar"]).toBe("#2b2d31");
    expect(t.vars["--background"]).toBe("#313338");
    expect(t.vars["--primary"]).toBe("#5865f2");
  });

  it("sets every shadcn token for every base theme", () => {
    const required = [
      "--background",
      "--foreground",
      "--card",
      "--popover",
      "--primary",
      "--primary-foreground",
      "--muted",
      "--muted-foreground",
      "--accent",
      "--border",
      "--input",
      "--ring",
      "--sidebar",
      "--destructive",
    ];
    for (const theme of Object.keys(BASE_THEMES) as (keyof typeof BASE_THEMES)[]) {
      const vars = buildTheme(ui({ theme })).vars;
      for (const k of required) expect(vars[k], `${theme} ${k}`).toBeTruthy();
    }
    expect(buildTheme(ui({ theme: "light" })).mode).toBe("light");
  });

  it("follows the accent, with readable text on it", () => {
    const t = buildTheme(ui({ accent: "#f0b232" }));
    expect(t.vars["--primary"]).toBe("#f0b232");
    expect(t.vars["--ring"]).toBe("#f0b232");
    expect(t.vars["--primary-foreground"]).toBe("#111214");
  });

  it("falls back from a malformed accent", () => {
    expect(buildTheme(ui({ accent: "nope" })).vars["--primary"]).toBe("#5865f2");
  });

  it("turns surfaces translucent over a gradient", () => {
    const sunset = GRADIENT_PRESETS.find((g) => g.name === "Sunset")!;
    const t = buildTheme(ui({ gradient: sunset }));
    expect(t.vars["--app-background"]).toBe("linear-gradient(135deg, #ff7a59, #8a2be2)");
    expect(t.vars["--background"]).toMatch(/^rgb\(0 0 0 \/ /);
    // Menus stay solid for readability.
    expect(t.vars["--popover"]).toBe("#111214");
  });

  it("uses light surfaces on light gradients", () => {
    const peach = GRADIENT_PRESETS.find((g) => g.name === "Peach")!;
    expect(gradientMode(peach)).toBe("light");
    expect(buildTheme(ui({ gradient: peach })).mode).toBe("light");
    expect(gradientMode(GRADIENT_PRESETS.find((g) => g.name === "Ocean")!)).toBe("dark");
  });
});
