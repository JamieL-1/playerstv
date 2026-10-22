import { Circle, Power, Scissors, Square } from "lucide-react";
import { toast } from "sonner";
import { backend } from "@/api/backend";
import type { Snapshot } from "@/api/types";
import { Button } from "@/components/ui/button";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { useNow } from "@/hooks/useStatus";
import { formatElapsed } from "@/lib/format";
import { cn } from "@/lib/utils";

/**
 * Bottom of the channel column, like Discord's "Voice Connected" panel:
 * what's being recorded right now, with the main actions.
 */
export function RecordingPanel({ snapshot }: { snapshot: Snapshot | null | undefined }) {
  const now = useNow();

  if (snapshot === undefined) return <Panel dot="bg-muted-foreground" title="Connecting…" />;
  if (snapshot === null) {
    return (
      <Panel dot="bg-muted-foreground" title="playerstv isn't running" subtitle="Nothing is being recorded">
        <IconButton label="Start playerstv" onClick={() => run(backend.startTray(), "Starting playerstv…")}>
          <Power />
        </IconButton>
      </Panel>
    );
  }

  const s = snapshot.status;
  const recording = s.state === "recording";
  const session = recording ? s.session_since : null;

  let dot = "bg-muted-foreground";
  let titleClass = "";
  let title = "Not recording";
  let subtitle = "Waiting for a game";
  if (s.state === "recording" && session !== null) {
    dot = "bg-red-500 animate-pulse";
    titleClass = "text-red-400";
    title = `REC ${formatElapsed(now - session)}`;
    subtitle = `${s.game} · full session`;
  } else if (s.state === "recording") {
    const ready = Math.min(now - s.since, snapshot.buffer_seconds);
    dot = "bg-green-500";
    titleClass = "text-green-400";
    title = "Replay buffer on";
    subtitle = `${s.game} · ${ready}s of ${snapshot.buffer_seconds}s ready`;
  } else if (s.state === "failing") {
    dot = "bg-amber-500";
    titleClass = "text-amber-400";
    title = "Encoder error";
    subtitle = `${s.game} · retrying`;
  }

  return (
    <Panel dot={dot} title={title} titleClass={titleClass} subtitle={subtitle}>
      <IconButton
        label={`Save last ${snapshot.buffer_seconds}s (${snapshot.clip_hotkey})`}
        disabled={!recording}
        onClick={() => run(backend.saveClip(), "Saving clip…")}
      >
        <Scissors />
      </IconButton>
      <IconButton
        label={
          session !== null
            ? `Stop & save session (${snapshot.session_hotkey})`
            : `Record full session (${snapshot.session_hotkey})`
        }
        disabled={!recording}
        destructive={session !== null}
        onClick={() => run(backend.toggleSession(), session !== null ? "Saving session…" : "Recording full session")}
      >
        {session !== null ? <Square className="fill-current" /> : <Circle />}
      </IconButton>
    </Panel>
  );
}

function run(p: Promise<unknown>, message: string) {
  toast.promise(p, { loading: message, success: message.replace("…", ""), error: (e) => String(e) });
}

function Panel(props: {
  dot: string;
  title: string;
  titleClass?: string;
  subtitle?: string;
  children?: React.ReactNode;
}) {
  const { dot, title, titleClass, subtitle, children } = props;
  return (
    <div className="flex items-center gap-2 border-t border-border bg-rail/60 px-2 py-2">
      <span className={cn("ml-1 size-2.5 shrink-0 rounded-full", dot)} />
      <div className="min-w-0 flex-1">
        <div className={cn("truncate text-sm font-semibold", titleClass)}>{title}</div>
        {subtitle && <div className="truncate text-xs text-muted-foreground">{subtitle}</div>}
      </div>
      <div className="flex shrink-0 gap-0.5">{children}</div>
    </div>
  );
}

function IconButton(props: {
  label: string;
  disabled?: boolean;
  destructive?: boolean;
  onClick: () => void;
  children: React.ReactNode;
}) {
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <span>
          <Button
            size="icon"
            variant="ghost"
            aria-label={props.label}
            disabled={props.disabled}
            onClick={props.onClick}
            className={cn("size-8", props.destructive && "text-red-400 hover:text-red-300")}
          >
            {props.children}
          </Button>
        </span>
      </TooltipTrigger>
      <TooltipContent>{props.label}</TooltipContent>
    </Tooltip>
  );
}
