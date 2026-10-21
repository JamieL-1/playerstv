import type { GameProfile } from "@/api/types";

/** Rail avatar for a game: short initials on a colour (no game artwork). */
export interface GameAvatar {
  initials: string;
  colour: string;
}

const BUILTIN: Record<string, GameAvatar> = {
  runelite: { initials: "RL", colour: "#c98a2c" },
  "league-of-legends": { initials: "LoL", colour: "#0a8f9e" },
  cs2: { initials: "CS2", colour: "#d1702a" },
};

const PALETTE = ["#5865f2", "#23a55a", "#eb459e", "#f23f43", "#9b59b6", "#00b0f4", "#e67e22", "#1abc9c"];

export function initials(name: string): string {
  const words = name
    .trim()
    .split(/[\s_\-:]+/)
    .filter(Boolean);
  if (words.length === 0) return "?";
  if (words.length === 1) return words[0]!.slice(0, 2).toUpperCase();
  return words
    .slice(0, 3)
    .map((w) => w[0]!.toUpperCase())
    .join("");
}

function hash(s: string): number {
  let h = 0;
  for (const c of s) h = (h * 31 + c.charCodeAt(0)) | 0;
  return Math.abs(h);
}

export function gameAvatar(game: Pick<GameProfile, "id" | "name">): GameAvatar {
  return BUILTIN[game.id] ?? { initials: initials(game.name), colour: PALETTE[hash(game.id) % PALETTE.length]! };
}

/** The folder name the Rust side uses for a game (profile::safe_name). */
export function safeName(name: string): string {
  const s = [...name].map((c) => (/[A-Za-z0-9_-]/.test(c) ? c : "_")).join("");
  return s || "Game";
}
