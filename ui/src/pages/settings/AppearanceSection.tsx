import { useQueryClient } from "@tanstack/react-query";
import { useRef } from "react";
import { Check } from "lucide-react";
import { toast } from "sonner";
import { backend } from "@/api/backend";
import type { BaseTheme, Config, Gradient, UiConfig } from "@/api/types";
import { CONFIG_KEY } from "@/hooks/useConfig";
import { cn } from "@/lib/utils";
import { useTheme } from "@/theme/ThemeProvider";
import { ACCENT_PRESETS, BASE_THEMES, GRADIENT_PRESETS } from "@/theme/themes";
import { GroupLabel, SectionTitle } from "./controls";

/** How long after the last change the theme is written to config.toml. */
const SAVE_DELAY_MS = 300;

/** Appearance changes apply instantly (and save shortly after), like Discord's. */
export function AppearanceSection() {
  const { ui } = useTheme();
  const client = useQueryClient();
  const pending = useRef<ReturnType<typeof setTimeout> | null>(null);

  // Show it now; write it once the user stops changing it (colour pickers
  // fire continuously while dragging, and each save makes the tray reload).
  const apply = (next: UiConfig) => {
    const current = client.getQueryData<Config>(CONFIG_KEY);
    if (current) client.setQueryData<Config>(CONFIG_KEY, { ...current, ui: next });
    if (pending.current) clearTimeout(pending.current);
    pending.current = setTimeout(() => {
      backend.setUiConfig(next).catch((e) => toast.error(String(e)));
    }, SAVE_DELAY_MS);
  };

  return (
    <>
      <SectionTitle title="Appearance" description="Changes apply straight away." />

      <GroupLabel>Theme</GroupLabel>
      <div className="grid grid-cols-4 gap-3">
        {(Object.keys(BASE_THEMES) as BaseTheme[]).map((t) => {
          const p = BASE_THEMES[t].palette;
          const active = !ui.gradient && ui.theme === t;
          return (
            <button
              key={t}
              onClick={() => apply({ ...ui, theme: t, gradient: null })}
              className={cn(
                "group relative overflow-hidden rounded-lg border-2 text-left transition-colors",
                active ? "border-primary" : "border-transparent hover:border-border",
              )}
            >
              <div className="flex h-16" style={{ background: p.background }}>
                <div className="w-4" style={{ background: p.rail }} />
                <div className="w-8" style={{ background: p.sidebar }} />
                <div className="m-2 h-2 w-10 rounded" style={{ background: p.muted }} />
              </div>
              <div className="bg-card px-2 py-1.5 text-sm font-medium">{BASE_THEMES[t].label}</div>
              {active && <ActiveBadge />}
            </button>
          );
        })}
      </div>

      <GroupLabel>Accent colour</GroupLabel>
      <div className="flex flex-wrap items-center gap-2">
        {ACCENT_PRESETS.map((a) => (
          <button
            key={a.hex}
            title={a.name}
            aria-label={a.name}
            onClick={() => apply({ ...ui, accent: a.hex })}
            className="relative flex size-10 items-center justify-center rounded-full ring-offset-2 ring-offset-background transition-transform hover:scale-110"
            style={{ background: a.hex }}
          >
            {ui.accent === a.hex && <Check className="size-5 text-white drop-shadow" />}
          </button>
        ))}
        <label
          className="relative flex size-10 cursor-pointer items-center justify-center overflow-hidden rounded-full border-2 border-dashed border-muted-foreground"
          title="Custom colour"
        >
          <span className="text-lg text-muted-foreground">+</span>
          <input
            type="color"
            className="absolute inset-0 cursor-pointer opacity-0"
            value={ui.accent}
            onChange={(e) => apply({ ...ui, accent: e.target.value })}
          />
        </label>
      </div>

      <GroupLabel>Gradient themes</GroupLabel>
      <div className="grid grid-cols-4 gap-3">
        {GRADIENT_PRESETS.map((g) => (
          <GradientSwatch
            key={g.name}
            gradient={g}
            active={ui.gradient?.name === g.name}
            onClick={() => apply({ ...ui, gradient: g })}
          />
        ))}
      </div>
      <div className="mt-4 flex flex-wrap items-center gap-3 rounded-lg bg-card p-3">
        <span className="text-sm font-medium">Custom gradient</span>
        <ColourInput
          label="From"
          value={ui.gradient?.name === "custom" ? ui.gradient.from : "#5865f2"}
          onChange={(from) => apply({ ...ui, gradient: { name: "custom", from, to: ui.gradient?.to ?? "#eb459e" } })}
        />
        <ColourInput
          label="To"
          value={ui.gradient?.name === "custom" ? ui.gradient.to : "#eb459e"}
          onChange={(to) => apply({ ...ui, gradient: { name: "custom", from: ui.gradient?.from ?? "#5865f2", to } })}
        />
        {ui.gradient && (
          <button
            className="ml-auto text-sm text-primary hover:underline"
            onClick={() => apply({ ...ui, gradient: null })}
          >
            Remove gradient
          </button>
        )}
      </div>
    </>
  );
}

function GradientSwatch({ gradient, active, onClick }: { gradient: Gradient; active: boolean; onClick: () => void }) {
  return (
    <button
      onClick={onClick}
      className={cn(
        "relative h-20 rounded-lg border-2 transition-transform hover:scale-[1.03]",
        active ? "border-primary" : "border-transparent",
      )}
      style={{ background: `linear-gradient(135deg, ${gradient.from}, ${gradient.to})` }}
    >
      <span className="absolute bottom-1.5 left-2 rounded bg-black/40 px-1.5 text-xs font-semibold text-white">
        {gradient.name}
      </span>
      {active && <ActiveBadge />}
    </button>
  );
}

function ColourInput({ label, value, onChange }: { label: string; value: string; onChange: (hex: string) => void }) {
  return (
    <label className="flex items-center gap-2 text-sm text-muted-foreground">
      {label}
      <input
        type="color"
        value={value}
        onChange={(e) => onChange(e.target.value)}
        className="h-8 w-12 cursor-pointer rounded border-0 bg-transparent"
      />
    </label>
  );
}

function ActiveBadge() {
  return (
    <span className="absolute right-1.5 top-1.5 flex size-5 items-center justify-center rounded-full bg-primary text-primary-foreground">
      <Check className="size-3.5" />
    </span>
  );
}
