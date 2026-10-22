import { useQuery } from "@tanstack/react-query";
import { backend } from "@/api/backend";
import { LIMITS, type AudioSourceKind } from "@/api/types";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Slider } from "@/components/ui/slider";
import { Switch } from "@/components/ui/switch";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import { Hint, SectionTitle, SettingRow } from "./controls";
import { OVERRIDE_NOTE } from "./options";
import { useSettings } from "./SettingsContext";

const DEFAULT_MIC = "__default__";

// The default "on" style is too subtle on Discord greys; use the accent.
const SELECTED = "data-[state=on]:bg-primary data-[state=on]:text-primary-foreground";

export function AudioSection() {
  const { draft, update } = useSettings();
  const a = draft.audio;
  const set = (patch: Partial<typeof a>) => update((c) => ({ ...c, audio: { ...c.audio, ...patch } }));
  const mics = useQuery({ queryKey: ["mics"], queryFn: backend.listMics, staleTime: 30_000 });
  const both = a.game_audio && a.mic;
  const source = a.source === "game" ? "Game" : "Desktop";
  const tracks =
    a.game_audio && a.mic
      ? a.mixed_track
        ? `1 Mixed · 2 ${source} · 3 Mic`
        : `1 ${source} · 2 Mic`
      : a.game_audio
        ? `1 ${source}`
        : a.mic
          ? "1 Mic"
          : "No audio";
  const [vMin, vMax] = LIMITS.volume;
  const [bMin, bMax] = LIMITS.audioBitrateKbps;

  return (
    <>
      <SectionTitle title="Audio" description={`Tracks in each recording: ${tracks}. ${OVERRIDE_NOTE}`} />
      <SettingRow label="Record computer audio">
        <Switch checked={a.game_audio} onCheckedChange={(v) => set({ game_audio: v })} />
      </SettingRow>
      <SettingRow label="What to record" disabled={!a.game_audio}>
        <ToggleGroup
          type="single"
          variant="outline"
          value={a.source}
          disabled={!a.game_audio}
          onValueChange={(v) => v && set({ source: v as AudioSourceKind })}
        >
          <ToggleGroupItem value="game" className={SELECTED}>
            Only the game
          </ToggleGroupItem>
          <ToggleGroupItem value="desktop" className={SELECTED}>
            All desktop audio
          </ToggleGroupItem>
        </ToggleGroup>
      </SettingRow>
      <SettingRow label="Record microphone">
        <Switch checked={a.mic} onCheckedChange={(v) => set({ mic: v })} />
      </SettingRow>
      <SettingRow label="Microphone" disabled={!a.mic}>
        <Select
          value={a.mic_device ?? DEFAULT_MIC}
          disabled={!a.mic}
          onValueChange={(m) => set({ mic_device: m === DEFAULT_MIC ? null : m })}
        >
          <SelectTrigger className="w-72">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value={DEFAULT_MIC}>Windows default</SelectItem>
            {(mics.data ?? []).map((m) => (
              <SelectItem key={m} value={m}>
                {m}
              </SelectItem>
            ))}
            {a.mic_device && !(mics.data ?? []).includes(a.mic_device) && (
              <SelectItem value={a.mic_device}>{a.mic_device} (not connected)</SelectItem>
            )}
          </SelectContent>
        </Select>
      </SettingRow>
      <SettingRow
        label="Add a mixed track first"
        description="Most players and YouTube only play the first audio track. With this on, track 1 mixes everything."
        disabled={!both}
      >
        <Switch checked={a.mixed_track} disabled={!both} onCheckedChange={(v) => set({ mixed_track: v })} />
      </SettingRow>
      {both && a.mixed_track && (
        <>
          <VolumeRow
            label={`${source} volume in the mix`}
            value={a.game_volume}
            min={vMin}
            max={vMax}
            onChange={(v) => set({ game_volume: v })}
          />
          <VolumeRow
            label="Mic volume in the mix"
            value={a.mic_volume}
            min={vMin}
            max={vMax}
            onChange={(v) => set({ mic_volume: v })}
          />
          <Hint>The separate tracks are always recorded untouched; these only change the mix.</Hint>
        </>
      )}
      <SettingRow label="Audio bitrate" stacked>
        <div className="flex items-center gap-4">
          <Slider
            className="flex-1"
            min={bMin}
            max={bMax}
            step={16}
            value={[a.bitrate_kbps]}
            onValueChange={([b]) => set({ bitrate_kbps: b ?? a.bitrate_kbps })}
          />
          <span className="w-20 text-right font-mono text-sm">{a.bitrate_kbps} kbps</span>
        </div>
      </SettingRow>
    </>
  );
}

function VolumeRow(props: { label: string; value: number; min: number; max: number; onChange: (v: number) => void }) {
  return (
    <SettingRow label={props.label} stacked>
      <div className="flex items-center gap-4">
        <Slider
          className="flex-1"
          min={props.min}
          max={props.max}
          step={5}
          value={[props.value]}
          onValueChange={([v]) => props.onChange(v ?? props.value)}
        />
        <span className="w-14 text-right font-mono text-sm">{props.value}%</span>
      </div>
    </SettingRow>
  );
}
