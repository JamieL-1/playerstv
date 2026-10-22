import { ArrowUpCircle } from "lucide-react";
import { toast } from "sonner";
import { backend } from "@/api/backend";
import type { Snapshot } from "@/api/types";
import { Button } from "@/components/ui/button";

/**
 * Shown above the recording panel once the tray has downloaded an update.
 * It installs by itself when playerstv closes; this restarts into it now.
 */
export function UpdateBanner({ snapshot }: { snapshot: Snapshot | null | undefined }) {
  const version = snapshot?.update_ready;
  if (!version) return null;
  const recording = snapshot.status.state === "recording";
  return (
    <div className="flex items-center gap-2 border-t border-border bg-rail/60 px-3 py-2">
      <ArrowUpCircle className="size-4 shrink-0 text-green-400" />
      <div className="min-w-0 flex-1">
        <div className="truncate text-sm font-semibold">Update v{version} ready</div>
        <div className="truncate text-xs text-muted-foreground">
          {recording ? "Restarting saves the recording first" : "Installs when playerstv closes"}
        </div>
      </div>
      <Button
        size="sm"
        variant="secondary"
        onClick={() => backend.restartToUpdate().catch((e) => toast.error(`Couldn't restart: ${String(e)}`))}
      >
        Restart
      </Button>
    </div>
  );
}
