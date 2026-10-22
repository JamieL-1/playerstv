import { useMemo } from "react";
import type { Codec, Quality, Resolution } from "@/api/types";

// Choices shared by the global Video section and each game's own settings.

export const RESOLUTIONS: { value: Resolution; label: string }[] = [
  { value: "native", label: "Native (window size)" },
  { value: "2x", label: "2× pixel-perfect (sharpest)" },
  { value: "720p", label: "720p" },
  { value: "1080p", label: "1080p" },
  { value: "1440p", label: "1440p" },
  { value: "2160p", label: "4K (2160p)" },
];

export const CODECS: { value: Codec; label: string; probe: string }[] = [
  { value: "h264", label: "H.264 (most compatible)", probe: 'video/mp4; codecs="avc1.640028"' },
  { value: "hevc", label: "HEVC / H.265 (smaller files)", probe: 'video/mp4; codecs="hvc1.1.6.L120.90"' },
  { value: "av1", label: "AV1 (smallest, RTX 40+)", probe: 'video/mp4; codecs="av01.0.08M.08"' },
];

export const QUALITIES: { value: Quality; label: string }[] = [
  { value: "low", label: "Low (small files)" },
  { value: "medium", label: "Medium" },
  { value: "high", label: "High" },
  { value: "very_high", label: "Very high" },
  { value: "max", label: "Maximum (big files)" },
];

export const labelOf = <T extends string>(options: { value: T; label: string }[], value: T): string =>
  options.find((o) => o.value === value)?.label ?? value;

/** Which codecs this PC's WebView can play back in the app's player. */
export function usePlayableCodecs(): Set<Codec> {
  return useMemo(() => {
    const v = document.createElement("video");
    return new Set(CODECS.filter((c) => v.canPlayType(c.probe) !== "").map((c) => c.value));
  }, []);
}

/** Under the global Recording/Video/Audio sections. */
export const OVERRIDE_NOTE = "Games can have their own settings: Games → Customize.";
