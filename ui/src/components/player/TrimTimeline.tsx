import { useRef, useState } from "react";
import { formatElapsed } from "@/lib/format";
import { dragHandle, pxToTime, snapToFrame, timeToFraction, type Range } from "@/lib/timeline";
import { cn } from "@/lib/utils";

type Drag = "start" | "end" | "seek";

/**
 * The trim editor's timeline: drag the in/out handles, click or drag
 * anywhere else to move the playhead. Handles snap to frames.
 */
export function TrimTimeline(props: {
  time: number;
  duration: number;
  fps: number;
  range: Range;
  onRangeChange: (r: Range) => void;
  onSeek: (t: number) => void;
}) {
  const { time, duration, fps, range, onRangeChange, onSeek } = props;
  const track = useRef<HTMLDivElement>(null);
  const [drag, setDrag] = useState<Drag | null>(null);

  const at = (clientX: number) => {
    const r = track.current!.getBoundingClientRect();
    return snapToFrame(pxToTime(clientX - r.left, r.width, duration), fps);
  };
  const move = (kind: Drag, clientX: number) => {
    const t = at(clientX);
    if (kind === "seek") onSeek(t);
    else {
      const next = dragHandle(range, kind, t, duration);
      onRangeChange(next);
      // Show the frame being cut at.
      onSeek(kind === "start" ? next.start : next.end);
    }
  };

  const left = timeToFraction(range.start, duration) * 100;
  const right = timeToFraction(range.end, duration) * 100;

  return (
    <div className="select-none">
      <div
        ref={track}
        className="relative h-14 cursor-pointer rounded-md bg-rail ring-1 ring-border"
        onPointerDown={(e) => {
          const kind = ((e.target as HTMLElement).dataset.handle as Drag | undefined) ?? "seek";
          e.currentTarget.setPointerCapture(e.pointerId);
          setDrag(kind);
          move(kind, e.clientX);
        }}
        onPointerMove={(e) => drag && move(drag, e.clientX)}
        onPointerUp={() => setDrag(null)}
      >
        {/* Outside the selection is dimmed. */}
        <div className="absolute inset-y-0 left-0 rounded-l-md bg-black/40" style={{ width: `${left}%` }} />
        <div className="absolute inset-y-0 right-0 rounded-r-md bg-black/40" style={{ width: `${100 - right}%` }} />
        <div
          className="pointer-events-none absolute inset-y-0 border-y-2 border-primary bg-primary/15"
          style={{ left: `${left}%`, right: `${100 - right}%` }}
        />
        <Handle side="start" at={left} active={drag === "start"} />
        <Handle side="end" at={right} active={drag === "end"} />
        <div
          className="pointer-events-none absolute -inset-y-1 w-0.5 -translate-x-1/2 rounded-full bg-white shadow"
          style={{ left: `${timeToFraction(time, duration) * 100}%` }}
        />
      </div>
      <div className="mt-1 flex justify-between text-xs tabular-nums text-muted-foreground">
        <span>{formatElapsed(0)}</span>
        <span>{formatElapsed(duration)}</span>
      </div>
    </div>
  );
}

function Handle({ side, at, active }: { side: "start" | "end"; at: number; active: boolean }) {
  return (
    <div
      data-handle={side}
      role="slider"
      aria-label={side === "start" ? "In point" : "Out point"}
      className={cn(
        "absolute inset-y-0 z-10 flex w-3 cursor-ew-resize items-center justify-center bg-primary transition-colors hover:brightness-110",
        side === "start" ? "-translate-x-full rounded-l-md" : "rounded-r-md",
        active && "brightness-125",
      )}
      style={{ left: `${at}%` }}
    >
      <span data-handle={side} className="h-5 w-0.5 rounded-full bg-primary-foreground/80" />
    </div>
  );
}
