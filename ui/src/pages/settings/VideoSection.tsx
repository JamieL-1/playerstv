import { LIMITS, type Codec, type Resolution } from "@/api/types";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Slider } from "@/components/ui/slider";
import { Switch } from "@/components/ui/switch";
import { Hint, SectionTitle, SettingRow } from "./controls";
import { CODECS, OVERRIDE_NOTE, QUALITIES, RESOLUTIONS, usePlayableCodecs } from "./options";
import { useSettings } from "./SettingsContext";

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
        description={`Encoded on your NVIDIA GPU (NVENC), so recording uses almost no CPU. ${OVERRIDE_NOTE}`}
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
          Every game pixel becomes a 2×2 block, so coloured text stays sharp in any player and YouTube serves it at 4K
          bitrates. Files are ~2.5× bigger. If the result is too big for the encoder, playerstv records at native size.
        </Hint>
      )}
      {v.resolution !== "native" && v.resolution !== "2x" && (
        <Hint>
          A size bigger than your game window only upscales it, though YouTube gives higher resolutions more bitrate.
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
                {!playable.has(c.value) && ", can't play on this PC"}
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
