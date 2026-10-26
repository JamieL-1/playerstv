import { RotateCcw } from "lucide-react";
import { toast } from "sonner";
import { backend } from "@/api/backend";
import type { CloseAction } from "@/api/types";
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
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import { restoreDefaults } from "@/lib/settingsDraft";
import { GroupLabel, SectionTitle, SettingRow } from "./controls";
import { useSettings } from "./SettingsContext";

const CLOSE_ACTIONS: { value: CloseAction; label: string }[] = [
  { value: "ask", label: "Ask every time" },
  { value: "tray", label: "Minimize to tray" },
  { value: "quit", label: "Quit playerstv" },
];

/** How the app itself behaves, as opposed to what it records. */
export function GeneralSection() {
  const { draft, update } = useSettings();
  return (
    <>
      <SectionTitle title="General" />
      <SettingRow label="Start playerstv with Windows" description="Starts in the tray, ready to record.">
        <Switch
          checked={draft.start_with_windows}
          onCheckedChange={(v) => update((c) => ({ ...c, start_with_windows: v }))}
        />
      </SettingRow>
      <SettingRow
        label="When I close the window"
        description="In the tray, playerstv keeps recording and the hotkeys keep working. Open the window again from the tray icon."
      >
        <Select
          value={draft.close_action}
          onValueChange={(a) => update((c) => ({ ...c, close_action: a as CloseAction }))}
        >
          <SelectTrigger className="w-52">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {CLOSE_ACTIONS.map((a) => (
              <SelectItem key={a.value} value={a.value}>
                {a.label}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </SettingRow>
      <SettingRow label="Notifications" description="Show a notification when a clip or session is saved.">
        <Switch checked={draft.notifications} onCheckedChange={(v) => update((c) => ({ ...c, notifications: v }))} />
      </SettingRow>

      <GroupLabel>Reset</GroupLabel>
      <SettingRow
        label="Restore default recording settings"
        description="Recording, video, audio, hotkeys and storage go back to how playerstv ships, and game customizations are cleared. Your games, output folder, appearance and the settings on this page stay."
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
      toast("Defaults restored. Click Save changes to keep them, or Reset to undo.");
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
          <AlertDialogTitle>Restore default recording settings?</AlertDialogTitle>
          <AlertDialogDescription>
            Recording, video, audio, hotkey and storage settings are reset and every game's customizations are cleared.
            Nothing changes until you click Save changes.
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
