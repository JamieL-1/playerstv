// Typed wrappers around the Rust commands in crates/ui/src.

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { Config, Snapshot, UiConfig } from "./types";

export const backend = {
  getStatus: () => invoke<Snapshot | null>("get_status"),
  saveClip: () => invoke<Snapshot>("save_clip"),
  toggleSession: () => invoke<Snapshot>("toggle_session"),
  startTray: () => invoke<void>("start_tray"),

  getConfig: () => invoke<Config>("get_config"),
  saveConfig: (config: Config) => invoke<Config>("save_config", { config }),
  setGameEnabled: (id: string, enabled: boolean) => invoke<void>("set_game_enabled", { id, enabled }),
  setUiConfig: (ui: UiConfig) => invoke<UiConfig>("set_ui_config", { ui }),
  validateHotkey: (hotkey: string) => invoke<boolean>("validate_hotkey", { hotkey }),
  listMics: () => invoke<string[]>("list_mics"),
  pickFolder: (start?: string) => invoke<string | null>("pick_folder", { start: start ?? null }),
  storageUsage: (dir: string) => invoke<{ bytes: number; files: number }>("storage_usage", { dir }),
};

export const events = {
  onStatus: (cb: (s: Snapshot | null) => void): Promise<UnlistenFn> =>
    listen<Snapshot | null>("status", (e) => cb(e.payload)),
  onConfigChanged: (cb: (c: Config) => void): Promise<UnlistenFn> =>
    listen<Config>("config-changed", (e) => cb(e.payload)),
};
