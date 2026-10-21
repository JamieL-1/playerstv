import { createContext, useContext, useEffect, useMemo, useState, type ReactNode } from "react";
import type { UiConfig } from "@/api/types";
import { useConfig } from "@/hooks/useConfig";
import { applyBuiltTheme, buildTheme } from "./themes";

const DEFAULT_UI: UiConfig = { theme: "dark", accent: "#5865f2", gradient: null };

interface ThemeContextValue {
  /** The theme on screen: a live preview if one is set, else the saved one. */
  ui: UiConfig;
  mode: "dark" | "light";
  /** Show a theme without saving it (Appearance settings), `null` to stop. */
  preview: (ui: UiConfig | null) => void;
}

const ThemeContext = createContext<ThemeContextValue | null>(null);

export function ThemeProvider({ children }: { children: ReactNode }) {
  const { data: config } = useConfig();
  const [previewUi, setPreviewUi] = useState<UiConfig | null>(null);
  const ui = previewUi ?? config?.ui ?? DEFAULT_UI;
  const theme = useMemo(() => buildTheme(ui), [ui]);

  useEffect(() => applyBuiltTheme(theme), [theme]);

  const value = useMemo(() => ({ ui, mode: theme.mode, preview: setPreviewUi }), [ui, theme.mode]);
  return <ThemeContext.Provider value={value}>{children}</ThemeContext.Provider>;
}

export function useTheme(): ThemeContextValue {
  const ctx = useContext(ThemeContext);
  if (!ctx) throw new Error("useTheme outside ThemeProvider");
  return ctx;
}
