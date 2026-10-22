import { useRef, useState } from "react";
import { formatElapsed } from "@/lib/format";
import { pxToTime, timeToFraction, type Range } from "@/lib/timeline";
import { cn } from "@/lib/utils";

/** The player's progress bar: click or drag to seek, hover for the time. */
export function SeekBar(props: {
  time: number;
  duration: number;
  /** Trim selection, shown as a band on the bar. */
  selection?: Range | null;
  onSeek: (t: number) => void;
}) {
  const { time, duration, selection, onSeek } = props;
  const bar = useRef<HTMLDivElement>(null);
  const [hover, setHover] = useState<number | null>(null);
  const [dragging, setDragging] = useState(false);

  const at = (clientX: number) => {
    const r = bar.current!.getBoundingClientRect();
    return pxToTime(clientX - r.left, r.width, duration);
  };

  return (
    <div
      ref={bar}
      role="slider"
      aria-label="Seek"
      aria-valuemin={0}
      aria-valuemax={duration}
      aria-valuenow={time}
      aria-valuetext={formatElapsed(time)}
      className="group/seek relative flex h-4 cursor-pointer items-center"
      onPointerDown={(e) => {
        e.currentTarget.setPointerCapture(e.pointerId);
        setDragging(true);
        onSeek(at(e.clientX));
      }}
      onPointerMove={(e) => {
        setHover(at(e.clientX));
        if (dragging) onSeek(at(e.clientX));
      }}
      onPointerUp={() => setDragging(false)}
      onPointerLeave={() => setHover(null)}
    >
      <div
        className={cn(
          "relative h-1 w-full overflow-hidden rounded-full bg-white/25 transition-[height] group-hover/seek:h-1.5",
          dragging && "h-1.5",
        )}
      >
        {selection && (
          <div
            className="absolute inset-y-0 bg-white/35"
            style={{
              left: `${timeToFraction(selection.start, duration) * 100}%`,
              right: `${(1 - timeToFraction(selection.end, duration)) * 100}%`,
            }}
          />
        )}
        <div
          className="absolute inset-y-0 left-0 bg-primary"
          style={{ width: `${timeToFraction(time, duration) * 100}%` }}
        />
      </div>
      <div
        className="pointer-events-none absolute size-3 -translate-x-1/2 rounded-full bg-primary opacity-0 shadow transition-opacity group-hover/seek:opacity-100"
        style={{ left: `${timeToFraction(time, duration) * 100}%` }}
      />
      {hover !== null && (
        <div
          className="pointer-events-none absolute bottom-5 -translate-x-1/2 rounded bg-black/85 px-1.5 py-0.5 text-xs font-medium tabular-nums text-white"
          style={{ left: `${timeToFraction(hover, duration) * 100}%` }}
        >
          {formatElapsed(hover)}
        </div>
      )}
    </div>
  );
}
