import { useQuery } from "@tanstack/react-query";
import { ArrowUpCircle, CheckCircle2, Loader2, RefreshCw, TriangleAlert } from "lucide-react";
import { useState, type ReactNode } from "react";
import { toast } from "sonner";
import { backend } from "@/api/backend";
import type { Snapshot } from "@/api/types";
import { Button } from "@/components/ui/button";
import { Switch } from "@/components/ui/switch";
import { useStatus } from "@/hooks/useStatus";
import { SectionTitle, SettingRow } from "./controls";
import { useSettings } from "./SettingsContext";

function formatTime(unixSeconds: number): string {
  return new Date(unixSeconds * 1000).toLocaleString([], { dateStyle: "medium", timeStyle: "short" });
}

export function UpdatesSection() {
  const { draft, update } = useSettings();
  const snapshot = useStatus();
  const version = useQuery({ queryKey: ["app-version"], queryFn: backend.appVersion, staleTime: Infinity });
  // Between clicking and the tray reporting a new state (up to half a
  // second). Cleared during render when the state changes, as React advises.
  const [requested, setRequested] = useState(false);
  const state = snapshot?.update;
  const stateKey = JSON.stringify(state);
  const [seenKey, setSeenKey] = useState(stateKey);
  if (stateKey !== seenKey) {
    setSeenKey(stateKey);
    setRequested(false);
  }

  const busy = requested || state?.state === "checking" || state?.state === "downloading";
  const canCheck = Boolean(state) && state?.state !== "unavailable" && state?.state !== "ready" && !busy;

  const check = async () => {
    setRequested(true);
    try {
      await backend.checkForUpdates();
    } catch (e) {
      setRequested(false);
      toast.error(`Couldn't check for updates: ${String(e)}`);
    }
  };

  return (
    <>
      <SectionTitle title="Updates" description="playerstv updates itself from its GitHub releases." />
      <div className="flex items-center gap-4 rounded-lg bg-card p-4">
        <div className="min-w-0 flex-1">
          <div className="font-semibold">playerstv {version.data ? `v${version.data}` : ""}</div>
          <div className="mt-1 text-sm text-muted-foreground">
            <UpdateLine snapshot={snapshot} requested={requested} />
          </div>
        </div>
        {state?.state === "ready" ? (
          <Button
            className="bg-green-600 text-white hover:bg-green-700"
            onClick={() => backend.restartToUpdate().catch((e) => toast.error(`Couldn't restart: ${String(e)}`))}
          >
            <ArrowUpCircle /> Restart to update
          </Button>
        ) : (
          <Button variant="secondary" disabled={!canCheck} onClick={() => void check()}>
            {busy ? <Loader2 className="animate-spin" /> : <RefreshCw />} Check for updates
          </Button>
        )}
      </div>
      <SettingRow
        label="Update automatically"
        description="Check every few hours and download new versions in the background. playerstv installs them when it isn't recording, or the next time you open it."
      >
        <Switch checked={draft.auto_update} onCheckedChange={(v) => update((c) => ({ ...c, auto_update: v }))} />
      </SettingRow>
    </>
  );
}

function UpdateLine({ snapshot, requested }: { snapshot: Snapshot | null | undefined; requested: boolean }) {
  if (snapshot === undefined) return null;
  if (snapshot === null) return <>playerstv isn't running in the tray, so it can't check for updates.</>;
  const s = snapshot.update;
  if (requested && s.state !== "checking" && s.state !== "downloading") return <Busy>Checking…</Busy>;
  switch (s.state) {
    case "unavailable":
      return <>This build doesn't update itself (development builds and test instances).</>;
    case "idle":
      return <>Not checked yet.</>;
    case "checking":
      return <Busy>Checking…</Busy>;
    case "downloading":
      return <Busy>Downloading v{s.version}…</Busy>;
    case "up_to_date":
      return (
        <span className="flex items-center gap-1.5">
          <CheckCircle2 className="size-4 text-green-500" /> You're up to date. Checked {formatTime(s.checked_at)}.
        </span>
      );
    case "ready": {
      const recording = snapshot.status.state === "recording";
      return (
        <>
          v{s.version} is downloaded.{" "}
          {recording ? "Restarting saves the recording first." : "It installs when playerstv closes, or restart now."}
        </>
      );
    }
    case "failed":
      return (
        <span className="flex items-start gap-1.5 text-amber-400">
          <TriangleAlert className="mt-0.5 size-4 shrink-0" />
          <span>
            Couldn't check for updates ({formatTime(s.at)}): {s.error}
          </span>
        </span>
      );
  }
}

function Busy({ children }: { children: ReactNode }) {
  return (
    <span className="flex items-center gap-1.5">
      <Loader2 className="size-4 animate-spin" /> {children}
    </span>
  );
}
