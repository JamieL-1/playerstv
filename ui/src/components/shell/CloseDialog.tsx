import { useEffect, useState } from "react";
import { toast } from "sonner";
import { backend, events } from "@/api/backend";
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
} from "@/components/ui/alert-dialog";
import { Checkbox } from "@/components/ui/checkbox";
import { Label } from "@/components/ui/label";
import { useStatus } from "@/hooks/useStatus";

/**
 * Asks what the close button should do, when the "When I close the window"
 * setting is "Ask every time" (crates/ui/src/status.rs::close_requested).
 */
export function CloseDialog() {
  const [open, setOpen] = useState(false);
  const [remember, setRemember] = useState(false);
  const status = useStatus();
  const session = status?.status.state === "recording" && status.status.session_since !== null;

  useEffect(() => {
    const unlisten = events.onCloseRequested(() => {
      setRemember(false);
      setOpen(true);
    });
    return () => {
      unlisten.then((f) => f());
    };
  }, []);

  const choose = async (action: Exclude<CloseAction, "ask">) => {
    try {
      if (remember) await backend.setCloseAction(action);
      await (action === "tray" ? backend.closeToTray() : backend.quitApp());
    } catch (e) {
      toast.error(String(e));
    }
  };

  return (
    <AlertDialog open={open} onOpenChange={setOpen}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>Close playerstv?</AlertDialogTitle>
          <AlertDialogDescription>
            Minimize to the tray to keep recording and the hotkeys working; open the window again from the tray icon.
            {session
              ? " Quitting saves the full session in progress, then stops recording."
              : " Quitting stops recording."}
          </AlertDialogDescription>
        </AlertDialogHeader>
        <div className="flex items-center gap-2">
          <Checkbox id="close-remember" checked={remember} onCheckedChange={(v) => setRemember(v === true)} />
          <Label htmlFor="close-remember" className="font-normal">
            Don't ask again (change this in Settings → General)
          </Label>
        </div>
        <AlertDialogFooter>
          <AlertDialogCancel>Cancel</AlertDialogCancel>
          <AlertDialogAction variant="destructive" onClick={() => void choose("quit")}>
            Quit
          </AlertDialogAction>
          <AlertDialogAction onClick={() => void choose("tray")}>Minimize to tray</AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}
