import { describe, expect, it } from "vitest";
import { formatElapsed, formatSize } from "./format";

describe("formatElapsed", () => {
  it("matches the tray's format", () => {
    expect(formatElapsed(0)).toBe("00:00");
    expect(formatElapsed(754)).toBe("12:34");
    expect(formatElapsed(3723)).toBe("1:02:03");
    expect(formatElapsed(-5)).toBe("00:00");
  });
});

describe("formatSize", () => {
  it("matches crates/core storage::format_size", () => {
    expect(formatSize(4_300_000_000)).toBe("4.3 GB");
    expect(formatSize(850_000_000)).toBe("850 MB");
  });
});
