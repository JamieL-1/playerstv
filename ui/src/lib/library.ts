import type { GameProfile, Recording } from "@/api/types";
import { safeName } from "./games";

export type KindFilter = "clips" | "sessions" | null;
export type SortOrder = "newest" | "oldest" | "largest" | "longest";

export const SORT_LABELS: Record<SortOrder, string> = {
  newest: "Newest first",
  oldest: "Oldest first",
  largest: "Largest first",
  longest: "Longest first",
};

export function parseKind(value: string | null): KindFilter {
  return value === "clips" || value === "sessions" ? value : null;
}

export interface LibraryFilter {
  /** Game folder (see gameFolder), or null for every game. */
  folder: string | null;
  kind: KindFilter;
  search: string;
}

/** The folder a game's recordings are saved in (profile::safe_name). */
export function gameFolder(game: Pick<GameProfile, "name">): string {
  return safeName(game.name);
}

/** A recording's game, by name if it's still configured. */
export function gameName(rec: Recording, games: GameProfile[]): string {
  return games.find((g) => gameFolder(g) === rec.game)?.name ?? rec.game.replaceAll("_", " ");
}

export function filterRecordings(recs: Recording[], { folder, kind, search }: LibraryFilter): Recording[] {
  const words = search.toLowerCase().split(/\s+/).filter(Boolean);
  return recs.filter((r) => {
    if (folder !== null && r.game !== folder) return false;
    if (kind === "clips" && r.kind !== "clip") return false;
    if (kind === "sessions" && r.kind !== "session") return false;
    const name = r.name.toLowerCase();
    return words.every((w) => name.includes(w));
  });
}

export function sortRecordings(recs: Recording[], order: SortOrder): Recording[] {
  const duration = (r: Recording) => r.info?.duration_s ?? 0;
  const by: Record<SortOrder, (a: Recording, b: Recording) => number> = {
    newest: (a, b) => b.modified - a.modified,
    oldest: (a, b) => a.modified - b.modified,
    largest: (a, b) => b.size - a.size,
    longest: (a, b) => duration(b) - duration(a),
  };
  // Ties (and the secondary order) go newest first.
  return [...recs].sort((a, b) => by[order](a, b) || b.modified - a.modified || b.name.localeCompare(a.name));
}

export interface Counts {
  all: number;
  clips: number;
  sessions: number;
}

export function countKinds(recs: Recording[], folder: string | null): Counts {
  const counts = { all: 0, clips: 0, sessions: 0 };
  for (const r of recs) {
    if (folder !== null && r.game !== folder) continue;
    counts.all++;
    if (r.kind === "clip") counts.clips++;
    else counts.sessions++;
  }
  return counts;
}

/** Whether the app's player can show it (WebView2 may lack HEVC). */
export function canPlay(rec: Recording, hevc: boolean = supportsHevc()): boolean {
  const v = rec.info?.video;
  if (!v?.playable) return false;
  return v.codec !== "hevc" || hevc;
}

let hevcSupport: boolean | undefined;
function supportsHevc(): boolean {
  if (hevcSupport === undefined) {
    hevcSupport =
      typeof document !== "undefined" &&
      document.createElement("video").canPlayType('video/mp4; codecs="hvc1.1.6.L120.90"') !== "";
  }
  return hevcSupport;
}

/** "Today at 14:03", "Yesterday at 14:03", "21/10/2026 14:03" (Discord-style). */
export function formatWhen(unixSeconds: number, now: Date = new Date()): string {
  const d = new Date(unixSeconds * 1000);
  const time = d.toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" });
  const startOfToday = new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime();
  const day = 24 * 3600 * 1000;
  if (d.getTime() >= startOfToday) return `Today at ${time}`;
  if (d.getTime() >= startOfToday - day) return `Yesterday at ${time}`;
  return `${d.toLocaleDateString()} ${time}`;
}

/** What a recording is called: the name the user gave it, else its file name without ".mp4". */
export function displayName(rec: Recording): string {
  return rec.custom_name ?? rec.name.replace(/\.mp4$/i, "");
}

/** The name a clip saved now would get by default (crates/core/src/clip.rs::output_path). */
export function defaultClipName(gameFolder: string, now: Date = new Date()): string {
  const p = (n: number) => String(n).padStart(2, "0");
  const date = `${now.getFullYear()}-${p(now.getMonth() + 1)}-${p(now.getDate())}`;
  return `${gameFolder}_${date}_${p(now.getHours())}-${p(now.getMinutes())}-${p(now.getSeconds())}`;
}

export const MAX_NAME_CHARS = 100;

/** Why `name` can't name a recording, or null if it can (crates/core/src/clip.rs::check_name). */
export function nameError(name: string): string | null {
  const trimmed = name
    .trim()
    .replace(/\.mp4$/i, "")
    .trimEnd();
  if (!trimmed) return "Enter a name";
  const chars = [...trimmed];
  if (chars.length > MAX_NAME_CHARS) return `Names can be at most ${MAX_NAME_CHARS} characters`;
  const control = (c: string) => c.charCodeAt(0) < 0x20 || (c.charCodeAt(0) >= 0x7f && c.charCodeAt(0) <= 0x9f);
  if (chars.some(control)) return "Names can't contain control characters";
  const bad = chars.find((c) => '<>:"/\\|?*'.includes(c));
  return bad ? `Names can't contain ${bad}` : null;
}
