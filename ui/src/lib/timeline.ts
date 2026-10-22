// Pure maths for the player's seek bar and the trim editor's timeline.

export interface Range {
  start: number;
  end: number;
}

/** Shortest selection the trim editor allows (seconds). */
export const MIN_SELECTION = 0.1;

export function clamp(v: number, lo: number, hi: number): number {
  return Math.min(hi, Math.max(lo, v));
}

/** Pointer x (relative to the bar) → time. */
export function pxToTime(x: number, width: number, duration: number): number {
  if (width <= 0 || duration <= 0) return 0;
  return clamp(x / width, 0, 1) * duration;
}

/** Time → fraction of the bar, 0–1. */
export function timeToFraction(t: number, duration: number): number {
  return duration > 0 ? clamp(t / duration, 0, 1) : 0;
}

/** Rounds to the nearest frame boundary (no-op without a frame rate). */
export function snapToFrame(t: number, fps: number): number {
  return fps > 0 ? Math.round(t * fps) / fps : t;
}

/**
 * Sets the in point. Past the out point, the out point resets to the end
 * (as in video editors) rather than the selection flipping.
 */
export function setIn(range: Range, t: number, duration: number): Range {
  const start = clamp(t, 0, Math.max(0, duration - MIN_SELECTION));
  const end = start > range.end - MIN_SELECTION ? duration : range.end;
  return { start, end };
}

/** Sets the out point; before the in point, the in point resets to 0. */
export function setOut(range: Range, t: number, duration: number): Range {
  const end = clamp(t, Math.min(MIN_SELECTION, duration), duration);
  const start = end < range.start + MIN_SELECTION ? 0 : range.start;
  return { start, end };
}

/** Dragging one handle: it stops at the other one instead of pushing it. */
export function dragHandle(range: Range, handle: "start" | "end", t: number, duration: number): Range {
  return handle === "start"
    ? { start: clamp(t, 0, range.end - MIN_SELECTION), end: range.end }
    : { start: range.start, end: clamp(t, range.start + MIN_SELECTION, duration) };
}

/** Where looped preview playback should be: back to the in point once past the out point. */
export function loopedTime(current: number, range: Range): number | null {
  return current >= range.end || current < range.start ? range.start : null;
}

/** "0:10.000", "1:02:03.500": precise times for in/out points. */
export function formatPrecise(seconds: number): string {
  const ms = Math.round(Math.max(0, seconds) * 1000);
  const h = Math.floor(ms / 3_600_000);
  const m = Math.floor(ms / 60_000) % 60;
  const s = Math.floor(ms / 1000) % 60;
  const frac = String(ms % 1000).padStart(3, "0");
  const ss = String(s).padStart(2, "0");
  return h > 0 ? `${h}:${String(m).padStart(2, "0")}:${ss}.${frac}` : `${m}:${ss}.${frac}`;
}
