import { FolderOpen, RotateCcw } from "lucide-react";
import { toast } from "sonner";
import { backend } from "@/api/backend";
import { LIMITS } from "@/api/types";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
  AlertDialogTrigger,
} from "@/components/ui/alert-dialog";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Slider } from "@/components/ui/slider";
import { Switch } from "@/components/ui/switch";
import { restoreDefaults } from "@/lib/settingsDraft";
import { HotkeyRecorder, SectionTitle, SettingRow } from "./controls";
import { OVERRIDE_NOTE } from "./options";
import { useSettings } from "./SettingsContext";

export function RecordingSection() {
  const { draft, update } = useSettings();
  const [min, max] = LIMITS.bufferSeconds;
  return (
    <>
      <SectionTitle title="Recording" description={`What gets recorded and where it's saved. ${OVERRIDE_NOTE}`} />
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
      <SettingRow label="Output folder" description="Clips and sessions are saved in a folder per game." stacked>
        <div className="flex gap-2">
          <Input
            value={draft.output_dir}
            onChange={(e) => update((c) => ({ ...c, output_dir: e.target.value }))}
            className="font-mono text-sm"
          />
          <Button
            variant="secondary"
            onClick={async () => {
              const dir = await backend.pickFolder(draft.output_dir);
              if (dir) update((c) => ({ ...c, output_dir: dir }));
            }}
          >
            <FolderOpen /> Browse
          </Button>
        </div>
      </SettingRow>
      <SettingRow label="Start playerstv with Windows">
        <Switch
          checked={draft.start_with_windows}
          onCheckedChange={(v) => update((c) => ({ ...c, start_with_windows: v }))}
        />
      </SettingRow>
      <SettingRow label="Notifications" description="Show a notification when a clip or session is saved.">
        <Switch checked={draft.notifications} onCheckedChange={(v) => update((c) => ({ ...c, notifications: v }))} />
      </SettingRow>
      <SettingRow
        label="Update automatically"
        description="Download new versions in the background. They're installed when playerstv isn't recording, or the next time you open it."
      >
        <Switch checked={draft.auto_update} onCheckedChange={(v) => update((c) => ({ ...c, auto_update: v }))} />
      </SettingRow>
      <SettingRow
        label="Restore default settings"
        description="Recording, video, audio, hotkeys and storage go back to how playerstv ships, and game customizations are cleared. Your games, output folder and appearance stay."
      >
        <RestoreDefaultsButton />
      </SettingRow>
    </>
  );
}

function RestoreDefaultsButton() {
  const { update } = useSettings();
  const restore = async () => {
    try {
      const defaults = await backend.defaultConfig();
      update((c) => restoreDefaults(c, defaults));
      toast("Defaults restored — Save Changes to keep them, or Reset to undo.");
    } catch (e) {
      toast.error(String(e));
    }
  };
  return (
    <AlertDialog>
      <AlertDialogTrigger asChild>
        <Button variant="secondary">
          <RotateCcw /> Restore defaults
        </Button>
      </AlertDialogTrigger>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>Restore default settings?</AlertDialogTitle>
          <AlertDialogDescription>
            Recording, video, audio, hotkey and storage settings are reset and every game's customizations are cleared.
            Nothing changes until you click Save Changes.
          </AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel>Cancel</AlertDialogCancel>
          <AlertDialogAction onClick={() => void restore()}>Restore defaults</AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
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
