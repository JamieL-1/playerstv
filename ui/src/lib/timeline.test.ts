import { describe, expect, it } from "vitest";
import {
  dragHandle,
  formatPrecise,
  loopedTime,
  MIN_SELECTION,
  pxToTime,
  setIn,
  setOut,
  snapToFrame,
  timeToFraction,
} from "./timeline";

describe("px ↔ time", () => {
  it("maps and clamps", () => {
    expect(pxToTime(50, 200, 60)).toBe(15);
    expect(pxToTime(-10, 200, 60)).toBe(0);
    expect(pxToTime(500, 200, 60)).toBe(60);
    expect(pxToTime(10, 0, 60)).toBe(0);
    expect(timeToFraction(15, 60)).toBe(0.25);
    expect(timeToFraction(99, 60)).toBe(1);
    expect(timeToFraction(1, 0)).toBe(0);
  });
});

describe("snapping", () => {
  it("rounds to frame boundaries", () => {
    expect(snapToFrame(10.008, 60)).toBeCloseTo(10.0, 6);
    expect(snapToFrame(10.009, 60)).toBeCloseTo(10 + 1 / 60, 6);
    expect(snapToFrame(1.234, 0)).toBe(1.234);
  });
});

describe("in/out points", () => {
  const all = { start: 0, end: 60 };

  it("sets in and out within the recording", () => {
    expect(setIn(all, 10, 60)).toEqual({ start: 10, end: 60 });
    expect(setOut({ start: 10, end: 60 }, 15, 60)).toEqual({ start: 10, end: 15 });
    expect(setIn(all, -5, 60).start).toBe(0);
    expect(setOut(all, 99, 60).end).toBe(60);
  });

  it("resets the other point instead of inverting the selection", () => {
    expect(setIn({ start: 10, end: 15 }, 20, 60)).toEqual({ start: 20, end: 60 });
    expect(setOut({ start: 10, end: 15 }, 5, 60)).toEqual({ start: 0, end: 5 });
    // In at the very end still leaves a minimal selection.
    expect(setIn(all, 60, 60)).toEqual({ start: 60 - MIN_SELECTION, end: 60 });
  });

  it("stops a dragged handle at the other one", () => {
    const r = { start: 10, end: 15 };
    expect(dragHandle(r, "start", 20, 60)).toEqual({ start: 15 - MIN_SELECTION, end: 15 });
    expect(dragHandle(r, "end", 2, 60)).toEqual({ start: 10, end: 10 + MIN_SELECTION });
    expect(dragHandle(r, "start", -1, 60)).toEqual({ start: 0, end: 15 });
    expect(dragHandle(r, "end", 70, 60)).toEqual({ start: 10, end: 60 });
  });

  it("loops preview playback inside the selection", () => {
    const r = { start: 10, end: 15 };
    expect(loopedTime(12, r)).toBeNull();
    expect(loopedTime(15, r)).toBe(10);
    expect(loopedTime(3, r)).toBe(10);
  });
});

describe("formatPrecise", () => {
  it("shows milliseconds", () => {
    expect(formatPrecise(10)).toBe("0:10.000");
    expect(formatPrecise(62.5)).toBe("1:02.500");
    expect(formatPrecise(3723.25)).toBe("1:02:03.250");
    expect(formatPrecise(-1)).toBe("0:00.000");
  });
});
