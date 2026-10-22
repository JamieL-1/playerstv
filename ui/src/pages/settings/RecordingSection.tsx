import { LIMITS } from "@/api/types";
import { Slider } from "@/components/ui/slider";
import { Switch } from "@/components/ui/switch";
import { HotkeyRecorder, SectionTitle, SettingRow } from "./controls";
import { OVERRIDE_NOTE } from "./options";
import { useSettings } from "./SettingsContext";

export function RecordingSection() {
  const { draft, update } = useSettings();
  const [min, max] = LIMITS.bufferSeconds;
  return (
    <>
      <SectionTitle title="Recording" description={`What gets recorded. ${OVERRIDE_NOTE}`} />
      <SettingRow
        label="Replay buffer length"
        description="How much of the last few minutes is kept, and saved when you clip."
        stacked
      >
        <div className="flex items-center gap-4">
          <Slider
            className="flex-1"
            min={min}
            max={max}
            step={5}
            value={[draft.buffer_seconds]}
            onValueChange={([v]) => update((c) => ({ ...c, buffer_seconds: v ?? c.buffer_seconds }))}
          />
          <span className="w-16 text-right font-mono text-sm">{draft.buffer_seconds} s</span>
        </div>
      </SettingRow>
      <SettingRow
        label="Record the whole session automatically"
        description="Saves everything from when the game opens until it closes, on top of the replay buffer."
      >
        <Switch
          checked={draft.record_full_session}
          onCheckedChange={(v) => update((c) => ({ ...c, record_full_session: v }))}
        />
      </SettingRow>
    </>
  );
}

export function HotkeysSection() {
  const { draft, update } = useSettings();
  return (
    <>
      <SectionTitle
        title="Hotkeys"
        description="Work everywhere, even while the game has focus. Click a hotkey, then press the new combination."
      />
      <SettingRow label="Save clip" description={`Saves the last ${draft.buffer_seconds} seconds.`}>
        <HotkeyRecorder value={draft.clip_hotkey} onChange={(h) => update((c) => ({ ...c, clip_hotkey: h }))} />
      </SettingRow>
      <SettingRow label="Start / stop full session">
        <HotkeyRecorder value={draft.session_hotkey} onChange={(h) => update((c) => ({ ...c, session_hotkey: h }))} />
      </SettingRow>
    </>
  );
}
