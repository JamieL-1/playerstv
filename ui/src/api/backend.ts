// Typed wrappers around the Rust commands in crates/ui/src.

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { Config, ExportDone, Recording, Snapshot, UiConfig } from "./types";

export const backend = {
  getStatus: () => invoke<Snapshot | null>("get_status"),
  saveClip: () => invoke<Snapshot>("save_clip"),
  toggleSession: () => invoke<Snapshot>("toggle_session"),
  restartToUpdate: () => invoke<Snapshot>("restart_to_update"),
  startTray: () => invoke<void>("start_tray"),
  quitApp: () => invoke<void>("quit_app"),

  getConfig: () => invoke<Config>("get_config"),
  saveConfig: (config: Config) => invoke<Config>("save_config", { config }),
  defaultConfig: () => invoke<Config>("default_config"),
  setGameEnabled: (id: string, enabled: boolean) => invoke<void>("set_game_enabled", { id, enabled }),
  setUiConfig: (ui: UiConfig) => invoke<UiConfig>("set_ui_config", { ui }),
  validateHotkey: (hotkey: string) => invoke<boolean>("validate_hotkey", { hotkey }),
  listMics: () => invoke<string[]>("list_mics"),
  pickFolder: (start?: string) => invoke<string | null>("pick_folder", { start: start ?? null }),
  gameIcons: () => invoke<Record<string, string>>("game_icons"),
  /** Picks an exe and adds it as a custom game; its id, or null if cancelled. */
  addGame: () => invoke<string | null>("add_game"),
  storageUsage: (dir: string) => invoke<{ bytes: number; files: number }>("storage_usage", { dir }),

  listRecordings: () => invoke<Recording[]>("list_recordings"),
  deleteRecording: (path: string) => invoke<void>("delete_recording", { path }),
  showInExplorer: (path: string) => invoke<void>("show_in_explorer", { path }),
  openExternal: (path: string) => invoke<void>("open_external", { path }),
  openOutputFolder: () => invoke<void>("open_output_folder"),
  exportTrim: (path: string, start: number, end: number) => invoke<number>("export_trim", { path, start, end }),
  cancelTrim: (id: number) => invoke<void>("cancel_trim", { id }),
};

export const events = {
  onStatus: (cb: (s: Snapshot | null) => void): Promise<UnlistenFn> =>
    listen<Snapshot | null>("status", (e) => cb(e.payload)),
  onConfigChanged: (cb: (c: Config) => void): Promise<UnlistenFn> =>
    listen<Config>("config-changed", (e) => cb(e.payload)),
  onLibraryChanged: (cb: () => void): Promise<UnlistenFn> => listen("library-changed", () => cb()),
  onThumbnailReady: (cb: (t: { path: string; thumb: string }) => void): Promise<UnlistenFn> =>
    listen<{ path: string; thumb: string }>("thumbnail-ready", (e) => cb(e.payload)),
  onExportProgress: (cb: (p: { id: number; fraction: number }) => void): Promise<UnlistenFn> =>
    listen<{ id: number; fraction: number }>("export-progress", (e) => cb(e.payload)),
  onExportDone: (cb: (d: ExportDone) => void): Promise<UnlistenFn> =>
    listen<ExportDone>("export-done", (e) => cb(e.payload)),
};
