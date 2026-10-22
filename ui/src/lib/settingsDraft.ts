import type { Config } from "@/api/types";

/** Deep equality for plain JSON values (configs). */
export function sameJson(a: unknown, b: unknown): boolean {
  return JSON.stringify(a) === JSON.stringify(b);
}

/**
 * config.toml changed on disk (tray menu, another window…) while the
 * settings form is open. Without unsaved edits, just take the new config.
 * With unsaved edits, keep them, but take the fields that are saved
 * instantly elsewhere (game switches, appearance) so a later Save can't
 * undo them.
 */
export function mergeExternalChange(draft: Config, saved: Config, incoming: Config): Config {
  if (sameJson(draft, saved)) return incoming;
  const enabled = new Map(incoming.games.map((g) => [g.id, g.enabled]));
  return {
    ...draft,
    games: draft.games.map((g) => (enabled.has(g.id) ? { ...g, enabled: enabled.get(g.id)! } : g)),
    ui: incoming.ui,
  };
}

export function isDirty(draft: Config | undefined, saved: Config | undefined): boolean {
  return Boolean(draft && saved && !sameJson(draft, saved));
}
