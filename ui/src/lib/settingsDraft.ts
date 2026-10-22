import type { Config } from "@/api/types";

/** Deep equality for plain JSON values (configs). */
export function sameJson(a: unknown, b: unknown): boolean {
  return JSON.stringify(a) === JSON.stringify(b);
}

/**
 * config.toml changed on disk (tray menu, another window…) while the
 * settings form is open. Without unsaved edits, just take the new config.
 * With unsaved edits, keep them, but take the fields that are saved
 * instantly elsewhere (game switches, appearance, the close dialog's
 * "Don't ask again") so a later Save can't undo them. A close action
 * edited here but not saved yet stays as edited.
 */
export function mergeExternalChange(draft: Config, saved: Config, incoming: Config): Config {
  if (sameJson(draft, saved)) return incoming;
  const enabled = new Map(incoming.games.map((g) => [g.id, g.enabled]));
  return {
    ...draft,
    games: draft.games.map((g) => (enabled.has(g.id) ? { ...g, enabled: enabled.get(g.id)! } : g)),
    ui: incoming.ui,
    close_action: draft.close_action === saved.close_action ? incoming.close_action : draft.close_action,
  };
}

/**
 * "Restore default recording settings": factory recording, video, audio,
 * hotkey and storage settings, and no per-game customizations. Keeps what
 * would hurt to lose: the games themselves (custom ones, on/off switches),
 * the output folder (the library reads it), the appearance, and the app
 * settings (General, Updates), which aren't recording settings.
 */
export function restoreDefaults(current: Config, defaults: Config): Config {
  return {
    ...defaults,
    output_dir: current.output_dir,
    ui: current.ui,
    start_with_windows: current.start_with_windows,
    notifications: current.notifications,
    auto_update: current.auto_update,
    close_action: current.close_action,
    games: current.games.map((g) => {
      const kept = { ...g };
      delete kept.overrides;
      return kept;
    }),
  };
}

export function isDirty(draft: Config | undefined, saved: Config | undefined): boolean {
  return Boolean(draft && saved && !sameJson(draft, saved));
}
