import { useMemo } from "react";
import { LIMITS, type Codec, type Quality, type Resolution } from "@/api/types";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Slider } from "@/components/ui/slider";
import { Switch } from "@/components/ui/switch";
import { Hint, SectionTitle, SettingRow } from "./controls";
import { useSettings } from "./SettingsContext";

const RESOLUTIONS: { value: Resolution; label: string }[] = [
  { value: "native", label: "Native (window size)" },
  { value: "2x", label: "2× pixel-perfect (sharpest)" },
  { value: "720p", label: "720p" },
  { value: "1080p", label: "1080p" },
  { value: "1440p", label: "1440p" },
  { value: "2160p", label: "4K (2160p)" },
];

const CODECS: { value: Codec; label: string; probe: string }[] = [
  { value: "h264", label: "H.264 (most compatible)", probe: 'video/mp4; codecs="avc1.640028"' },
  { value: "hevc", label: "HEVC / H.265 (smaller files)", probe: 'video/mp4; codecs="hvc1.1.6.L120.90"' },
  { value: "av1", label: "AV1 (smallest, RTX 40+)", probe: 'video/mp4; codecs="av01.0.08M.08"' },
];

const QUALITIES: { value: Quality; label: string }[] = [
  { value: "low", label: "Low (small files)" },
  { value: "medium", label: "Medium" },
  { value: "high", label: "High" },
  { value: "very_high", label: "Very high" },
  { value: "max", label: "Maximum (big files)" },
];

/** Which codecs this PC's WebView can play back in the app's player. */
function usePlayableCodecs(): Set<Codec> {
  return useMemo(() => {
    const v = document.createElement("video");
    return new Set(CODECS.filter((c) => v.canPlayType(c.probe) !== "").map((c) => c.value));
  }, []);
}

export function VideoSection() {
  const { draft, update } = useSettings();
  const v = draft.video;
  const playable = usePlayableCodecs();
  const set = (patch: Partial<typeof v>) => update((c) => ({ ...c, video: { ...c.video, ...patch } }));
  const qualityIndex = Math.max(
    0,
    QUALITIES.findIndex((q) => q.value === v.quality),
  );
  const [capMin, capMax] = LIMITS.bitrateCapMbps;

  return (
    <>
      <SectionTitle
        title="Video"
        description="Encoded on your NVIDIA GPU (NVENC), so recording costs almost nothing."
      />
      <SettingRow label="Resolution">
        <Select value={v.resolution} onValueChange={(r) => set({ resolution: r as Resolution })}>
          <SelectTrigger className="w-64">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {RESOLUTIONS.map((r) => (
              <SelectItem key={r.value} value={r.value}>
                {r.label}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </SettingRow>
      {v.resolution === "2x" && (
        <Hint>
          Every game pixel becomes a crisp 2×2 block: sharp coloured text in any player, and YouTube serves it at 4K
          bitrates. Files are ~2.5× bigger. Falls back to native if the result is too big for the encoder.
        </Hint>
      )}
      {v.resolution !== "native" && v.resolution !== "2x" && (
        <Hint>
          A size bigger than your game window just upscales it (YouTube does give higher resolutions more bitrate).
        </Hint>
      )}
      <SettingRow label="Frame rate">
        <Select value={String(v.fps)} onValueChange={(f) => set({ fps: Number(f) })}>
          <SelectTrigger className="w-64">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {LIMITS.fps.map((f) => (
              <SelectItem key={f} value={String(f)}>
                {f} fps
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </SettingRow>
      <SettingRow label="Codec" description="Everything recorded here plays in playerstv's own player.">
        <Select value={v.codec} onValueChange={(c) => set({ codec: c as Codec })}>
          <SelectTrigger className="w-64">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {CODECS.map((c) => (
              <SelectItem key={c.value} value={c.value} disabled={!playable.has(c.value)}>
                {c.label}
                {!playable.has(c.value) && " — can't play on this PC"}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </SettingRow>
      <SettingRow label="Quality" stacked>
        <div className="flex items-center gap-4">
          <Slider
            className="flex-1"
            min={0}
            max={QUALITIES.length - 1}
            step={1}
            value={[qualityIndex]}
            onValueChange={([i]) => set({ quality: QUALITIES[i ?? qualityIndex]!.value })}
          />
          <span className="w-40 text-right text-sm">{QUALITIES[qualityIndex]!.label}</span>
        </div>
      </SettingRow>
      <SettingRow
        label="Automatic bitrate cap"
        description="Picks a peak bitrate from the quality, resolution and frame rate."
      >
        <Switch
          checked={v.bitrate_cap_mbps === null}
          onCheckedChange={(auto) => set({ bitrate_cap_mbps: auto ? null : 40 })}
        />
      </SettingRow>
      {v.bitrate_cap_mbps !== null && (
        <SettingRow label="Bitrate cap" stacked>
          <div className="flex items-center gap-4">
            <Slider
              className="flex-1"
              min={capMin}
              max={capMax}
              step={1}
              value={[v.bitrate_cap_mbps]}
              onValueChange={([m]) => set({ bitrate_cap_mbps: m ?? v.bitrate_cap_mbps })}
            />
            <span className="w-24 text-right font-mono text-sm">{v.bitrate_cap_mbps} Mbps</span>
          </div>
        </SettingRow>
      )}
    </>
  );
}
