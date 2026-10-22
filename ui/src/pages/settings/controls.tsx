import { useEffect, useState, type ReactNode } from "react";
import { Button } from "@/components/ui/button";
import { Separator } from "@/components/ui/separator";
import { hotkeyFromKeyPress, hotkeyParts } from "@/lib/hotkeys";
import { cn } from "@/lib/utils";

export function SectionTitle({ title, description }: { title: string; description?: string }) {
  return (
    <div className="mb-5">
      <h1 className="text-xl font-semibold">{title}</h1>
      {description && <p className="mt-1 text-sm text-muted-foreground">{description}</p>}
    </div>
  );
}

/** Small uppercase heading inside a section, like Discord's. */
export function GroupLabel({ children }: { children: ReactNode }) {
  return <div className="mb-2 mt-6 text-xs font-bold uppercase tracking-wide text-muted-foreground">{children}</div>;
}

/** A labelled setting with its control on the right (or below when `stacked`). */
export function SettingRow(props: {
  label: string;
  description?: ReactNode;
  children: ReactNode;
  stacked?: boolean;
  disabled?: boolean;
}) {
  const { label, description, children, stacked, disabled } = props;
  return (
    <>
      <div className={cn("py-4", stacked ? "space-y-3" : "flex items-center gap-6", disabled && "opacity-50")}>
        <div className="min-w-0 flex-1">
          <div className="font-medium">{label}</div>
          {description && <div className="mt-0.5 text-sm text-muted-foreground">{description}</div>}
        </div>
        <div className={cn(stacked ? "" : "shrink-0")}>{children}</div>
      </div>
      <Separator />
    </>
  );
}

export function Hint({ tone = "muted", children }: { tone?: "muted" | "warn"; children: ReactNode }) {
  return <p className={cn("mt-2 text-xs", tone === "warn" ? "text-amber-400" : "text-muted-foreground")}>{children}</p>;
}

export function KeyCaps({ hotkey }: { hotkey: string }) {
  const parts = hotkeyParts(hotkey);
  if (parts.length === 0) return <span className="text-muted-foreground">Not set</span>;
  return (
    <span className="flex items-center gap-1">
      {parts.map((p, i) => (
        <kbd
          key={i}
          className="rounded-md border border-border bg-rail px-2 py-0.5 font-mono text-xs font-semibold shadow-[inset_0_-2px_0_rgb(0_0_0/0.35)]"
        >
          {p}
        </kbd>
      ))}
    </span>
  );
}

/** Click, then press the combination; Esc cancels, Backspace clears. */
export function HotkeyRecorder({ value, onChange }: { value: string; onChange: (hotkey: string) => void }) {
  const [recording, setRecording] = useState(false);
  const [message, setMessage] = useState<string | null>(null);

  useEffect(() => {
    if (!recording) return;
    const onKey = (e: KeyboardEvent) => {
      e.preventDefault();
      e.stopPropagation();
      if (e.code === "Escape") {
        setRecording(false);
        setMessage(null);
        return;
      }
      if (e.code === "Backspace" && !e.ctrlKey && !e.altKey && !e.shiftKey && !e.metaKey) {
        onChange("");
        setRecording(false);
        setMessage(null);
        return;
      }
      const result = hotkeyFromKeyPress(e);
      if (result.kind === "pending") return;
      if (result.kind === "needs-modifier") {
        setMessage(`Add Ctrl, Alt or Shift. ${result.key} on its own would stop working in every other app.`);
        return;
      }
      onChange(result.hotkey);
      setRecording(false);
      setMessage(null);
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [recording, onChange]);

  return (
    <div className="flex flex-col items-end gap-1">
      <Button
        variant={recording ? "default" : "secondary"}
        className="min-w-44 justify-center"
        onClick={() => {
          setMessage(null);
          setRecording((r) => !r);
        }}
      >
        {recording ? "Press keys… (Esc to cancel)" : <KeyCaps hotkey={value} />}
      </Button>
      {message && <span className="max-w-72 text-right text-xs text-amber-400">{message}</span>}
    </div>
  );
}
