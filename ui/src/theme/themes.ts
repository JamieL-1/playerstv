// Discord-style themes expressed as shadcn CSS-variable tokens. Every shadcn
// component (and our own layout) reads these, so swapping the values is all a
// theme change takes. Pure functions here; applyBuiltTheme() touches the DOM.

import type { BaseTheme, Gradient, UiConfig } from "@/api/types";

type Mode = "dark" | "light";

/** Surface colours, from the outermost (server rail) inwards. */
interface Palette {
  mode: Mode;
  rail: string; // server rail
  sidebar: string; // channel column
  background: string; // main panel
  card: string;
  popover: string;
  hover: string; // list-item hover (shadcn "accent")
  selected: string; // selected list item
  input: string;
  foreground: string;
  muted: string; // secondary text
  border: string;
}

export const BASE_THEMES: Record<BaseTheme, { label: string; palette: Palette }> = {
  dark: {
    label: "Dark",
    palette: {
      mode: "dark",
      rail: "#1e1f22",
      sidebar: "#2b2d31",
      background: "#313338",
      card: "#2b2d31",
      popover: "#111214",
      hover: "#35373c",
      selected: "#404249",
      input: "#1e1f22",
      foreground: "#dbdee1",
      muted: "#949ba4",
      border: "#ffffff14",
    },
  },
  ash: {
    label: "Ash",
    palette: {
      mode: "dark",
      rail: "#26272b",
      sidebar: "#2f3035",
      background: "#38393e",
      card: "#2f3035",
      popover: "#1c1d20",
      hover: "#404146",
      selected: "#4a4b51",
      input: "#26272b",
      foreground: "#e3e5e8",
      muted: "#a2a7b0",
      border: "#ffffff17",
    },
  },
  onyx: {
    label: "Onyx",
    palette: {
      mode: "dark",
      rail: "#060607",
      sidebar: "#0f0f11",
      background: "#151518",
      card: "#101012",
      popover: "#000000",
      hover: "#1e1e22",
      selected: "#27272c",
      input: "#060607",
      foreground: "#d8d9dc",
      muted: "#8a8f98",
      border: "#ffffff12",
    },
  },
  light: {
    label: "Light",
    palette: {
      mode: "light",
      rail: "#e3e5e8",
      sidebar: "#f2f3f5",
      background: "#ffffff",
      card: "#f2f3f5",
      popover: "#ffffff",
      hover: "#e8e9eb",
      selected: "#dcdee1",
      input: "#e3e5e8",
      foreground: "#313338",
      muted: "#5c5e66",
      border: "#0000001a",
    },
  },
};

export const ACCENT_PRESETS: { name: string; hex: string }[] = [
  { name: "Blurple", hex: "#5865f2" },
  { name: "Green", hex: "#23a55a" },
  { name: "Yellow", hex: "#f0b232" },
  { name: "Fuchsia", hex: "#eb459e" },
  { name: "Red", hex: "#f23f43" },
  { name: "Orange", hex: "#ff7a1a" },
  { name: "Cyan", hex: "#00b0f4" },
  { name: "Purple", hex: "#9b59b6" },
];

/** Nitro-style gradient themes (our own presets). */
export const GRADIENT_PRESETS: Gradient[] = [
  { name: "Midnight", from: "#2b1055", to: "#7597de" },
  { name: "Sunset", from: "#ff7a59", to: "#8a2be2" },
  { name: "Aurora", from: "#00c9a7", to: "#4458f2" },
  { name: "Ember", from: "#f83600", to: "#6a0572" },
  { name: "Forest", from: "#134e5e", to: "#71b280" },
  { name: "Ocean", from: "#0f2027", to: "#2c5364" },
  { name: "Peach", from: "#ffecd2", to: "#fcb69f" },
  { name: "Lavender", from: "#e0c3fc", to: "#8ec5fc" },
];

const HEX = /^#[0-9a-f]{6}$/i;

export function hexToRgb(hex: string): [number, number, number] {
  if (!HEX.test(hex)) throw new Error(`not a #rrggbb colour: ${hex}`);
  const n = parseInt(hex.slice(1), 16);
  return [(n >> 16) & 255, (n >> 8) & 255, n & 255];
}

/** WCAG relative luminance, 0 (black) … 1 (white). */
export function luminance(hex: string): number {
  const [r, g, b] = hexToRgb(hex).map((c) => {
    const s = c / 255;
    return s <= 0.03928 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
  }) as [number, number, number];
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

/** Black or white text, whichever reads better on `bg`. */
export function readableOn(bg: string): string {
  return luminance(bg) > 0.45 ? "#111214" : "#ffffff";
}

/** Light gradients get light translucent surfaces, dark ones dark. */
export function gradientMode(g: Gradient): Mode {
  return (luminance(g.from) + luminance(g.to)) / 2 > 0.4 ? "light" : "dark";
}

function overlay(mode: Mode, alpha: number): string {
  return mode === "dark" ? `rgb(0 0 0 / ${alpha})` : `rgb(255 255 255 / ${alpha})`;
}

/** Translucent surfaces over a gradient, like Nitro themes. */
function gradientPalette(g: Gradient): Palette {
  const mode = gradientMode(g);
  const base = mode === "dark" ? BASE_THEMES.dark.palette : BASE_THEMES.light.palette;
  return {
    ...base,
    mode,
    rail: overlay(mode, mode === "dark" ? 0.55 : 0.55),
    sidebar: overlay(mode, mode === "dark" ? 0.4 : 0.45),
    background: overlay(mode, mode === "dark" ? 0.25 : 0.3),
    card: overlay(mode, mode === "dark" ? 0.3 : 0.5),
    // Menus/dialogs stay solid so text over them is always readable.
    popover: mode === "dark" ? "#111214" : "#ffffff",
    hover: overlay(mode, mode === "dark" ? 0.18 : 0.35),
    selected: overlay(mode, mode === "dark" ? 0.3 : 0.55),
    input: overlay(mode, mode === "dark" ? 0.35 : 0.6),
  };
}

export interface BuiltTheme {
  mode: Mode;
  /** CSS custom properties to set on :root. */
  vars: Record<string, string>;
}

export function buildTheme(ui: UiConfig): BuiltTheme {
  const p = ui.gradient ? gradientPalette(ui.gradient) : BASE_THEMES[ui.theme].palette;
  const accent = HEX.test(ui.accent) ? ui.accent : "#5865f2";
  const onAccent = readableOn(accent);
  const destructive = "#da373c";
  const vars: Record<string, string> = {
    "--rail": p.rail,
    "--background": p.background,
    "--foreground": p.foreground,
    "--card": p.card,
    "--card-foreground": p.foreground,
    "--popover": p.popover,
    "--popover-foreground": p.foreground,
    "--primary": accent,
    "--primary-foreground": onAccent,
    "--secondary": p.hover,
    "--secondary-foreground": p.foreground,
    "--muted": p.hover,
    "--muted-foreground": p.muted,
    "--accent": p.hover,
    "--accent-foreground": p.foreground,
    "--selected": p.selected,
    "--destructive": destructive,
    "--border": p.border,
    "--input": p.input,
    "--ring": accent,
    "--sidebar": p.sidebar,
    "--sidebar-foreground": p.foreground,
    "--sidebar-primary": accent,
    "--sidebar-primary-foreground": onAccent,
    "--sidebar-accent": p.hover,
    "--sidebar-accent-foreground": p.foreground,
    "--sidebar-border": p.border,
    "--sidebar-ring": accent,
    "--app-background": ui.gradient ? `linear-gradient(135deg, ${ui.gradient.from}, ${ui.gradient.to})` : p.rail,
  };
  return { mode: p.mode, vars };
}

/** Applies a built theme to the document (and the `dark` class shadcn uses). */
export function applyBuiltTheme(theme: BuiltTheme): void {
  const root = document.documentElement;
  for (const [k, v] of Object.entries(theme.vars)) root.style.setProperty(k, v);
  root.classList.toggle("dark", theme.mode === "dark");
}
