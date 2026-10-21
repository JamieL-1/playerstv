import { useQueryClient } from "@tanstack/react-query";
import { createContext, useCallback, useContext, useMemo, useState, type ReactNode } from "react";
import { toast } from "sonner";
import { backend } from "@/api/backend";
import type { Config } from "@/api/types";
import { CONFIG_KEY, useConfig } from "@/hooks/useConfig";
import { isDirty, mergeExternalChange } from "@/lib/settingsDraft";

interface SettingsContextValue {
  draft: Config;
  saved: Config;
  dirty: boolean;
  saving: boolean;
  /** Edit the draft (saved with the Save button). */
  update: (fn: (c: Config) => Config) => void;
  save: () => Promise<void>;
  reset: () => void;
  /** Applied and saved immediately (like the tray menu does). */
  setGameEnabled: (id: string, enabled: boolean) => Promise<void>;
}

const SettingsContext = createContext<SettingsContextValue | null>(null);

export function SettingsProvider({ children }: { children: ReactNode }) {
  const { data: saved } = useConfig();
  const client = useQueryClient();
  // The saved config the draft was last reconciled with, and the draft.
  const [base, setBase] = useState<Config | undefined>(saved);
  const [draft, setDraft] = useState<Config | undefined>(saved);
  const [saving, setSaving] = useState(false);

  // The saved config changed (our save, the tray menu, another window):
  // follow it without losing unsaved edits. Done during render, React's
  // recommended way to adjust state when an input changes.
  if (saved && saved !== base) {
    setDraft(draft && base ? mergeExternalChange(draft, base, saved) : saved);
    setBase(saved);
  }

  const update = useCallback((fn: (c: Config) => Config) => setDraft((d) => (d ? fn(d) : d)), []);
  const reset = useCallback(() => setDraft(base), [base]);

  const save = useCallback(async () => {
    if (!draft) return;
    setSaving(true);
    try {
      const result = await backend.saveConfig(draft);
      setBase(result);
      setDraft(result);
      client.setQueryData(CONFIG_KEY, result);
      toast.success("Settings saved");
    } catch (e) {
      toast.error(String(e));
    } finally {
      setSaving(false);
    }
  }, [draft, client]);

  const setGameEnabled = useCallback(
    async (id: string, enabled: boolean) => {
      try {
        await backend.setGameEnabled(id, enabled);
        // The config watcher also reports this; updating the cache now just
        // makes the switch respond instantly.
        const current = client.getQueryData<Config>(CONFIG_KEY);
        if (current) {
          client.setQueryData<Config>(CONFIG_KEY, {
            ...current,
            games: current.games.map((g) => (g.id === id ? { ...g, enabled } : g)),
          });
        }
      } catch (e) {
        toast.error(String(e));
      }
    },
    [client],
  );

  const value = useMemo(
    () =>
      draft && base
        ? { draft, saved: base, dirty: isDirty(draft, base), saving, update, save, reset, setGameEnabled }
        : null,
    [draft, base, saving, update, save, reset, setGameEnabled],
  );
  if (!value) return null;
  return <SettingsContext.Provider value={value}>{children}</SettingsContext.Provider>;
}

export function useSettings(): SettingsContextValue {
  const ctx = useContext(SettingsContext);
  if (!ctx) throw new Error("useSettings outside SettingsProvider");
  return ctx;
}
