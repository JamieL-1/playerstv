import { useQuery } from "@tanstack/react-query";
import { FolderOpen } from "lucide-react";
import { backend } from "@/api/backend";
import { LIMITS } from "@/api/types";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Progress } from "@/components/ui/progress";
import { Slider } from "@/components/ui/slider";
import { Switch } from "@/components/ui/switch";
import { formatSize } from "@/lib/format";
import { Hint, SectionTitle, SettingRow } from "./controls";
import { useSettings } from "./SettingsContext";

/** Friendly slider stops; the number box takes any value in range. */
const STEPS_GB = [1, 2, 5, 10, 20, 50, 100, 200, 500, 1000, 2000, 5000, 10000, 20000];

function nearestStep(gb: number): number {
  let best = 0;
  STEPS_GB.forEach((s, i) => {
    if (Math.abs(Math.log(s) - Math.log(gb)) < Math.abs(Math.log(STEPS_GB[best]!) - Math.log(gb))) best = i;
  });
  return best;
}

export function StorageSection() {
  const { draft, update } = useSettings();
  const limit = draft.storage_limit_gb;
  const usage = useQuery({
    queryKey: ["storage-usage", draft.output_dir],
    queryFn: () => backend.storageUsage(draft.output_dir),
    refetchInterval: 3000,
  });
  const used = usage.data?.bytes ?? 0;
  const [min, max] = LIMITS.storageLimitGb;
  const set = (gb: number | null) => update((c) => ({ ...c, storage_limit_gb: gb }));

  return (
    <>
      <SectionTitle title="Storage" description="Where recordings are saved, and how much space they can use." />
      <SettingRow label="Output folder" description="playerstv saves clips and sessions in a folder per game." stacked>
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
      <div className="mb-2 mt-4 rounded-lg bg-card p-4">
        <div className="mb-2 flex justify-between text-sm">
          <span>
            Using <b>{formatSize(used)}</b>
            {limit !== null && ` of ${limit} GB`}
          </span>
          <span className="text-muted-foreground">{usage.data?.files ?? 0} recordings</span>
        </div>
        {limit !== null && <Progress value={Math.min(100, (used / (limit * 1e9)) * 100)} />}
      </div>
      <SettingRow
        label="Limit how much space recordings can use"
        description="When saved clips and sessions go over the limit, playerstv deletes the oldest."
      >
        <Switch checked={limit !== null} onCheckedChange={(on) => set(on ? (limit ?? 50) : null)} />
      </SettingRow>
      {limit !== null && (
        <>
          <SettingRow label="Maximum" stacked>
            <div className="flex items-center gap-4">
              <Slider
                className="flex-1"
                min={0}
                max={STEPS_GB.length - 1}
                step={1}
                value={[nearestStep(limit)]}
                onValueChange={([i]) => set(STEPS_GB[i ?? 0]!)}
              />
              <div className="flex items-center gap-2">
                <Input
                  type="number"
                  className="w-24 text-right font-mono"
                  min={min}
                  max={max}
                  value={limit}
                  onChange={(e) => {
                    const n = Math.round(Number(e.target.value));
                    if (Number.isFinite(n)) set(Math.min(max, Math.max(min, n)));
                  }}
                />
                <span className="text-sm">GB</span>
              </div>
            </div>
          </SettingRow>
          <SettingRow
            label="Keep named recordings"
            description="Clips and sessions you've given a name are never deleted to make space. They still count toward the limit."
          >
            <Switch
              checked={draft.keep_named_recordings}
              onCheckedChange={(on) => update((c) => ({ ...c, keep_named_recordings: on }))}
            />
          </SettingRow>
          <Hint tone="warn">
            playerstv deletes the oldest recordings permanently. Only files playerstv saved count;{" "}
            {draft.keep_named_recordings ? "name a recording or move it" : "move a recording"} out of the folder to keep
            it forever.
          </Hint>
          {used > limit * 1e9 && (
            <Hint tone="warn">You're over this limit. Saving will delete the oldest recordings now.</Hint>
          )}
        </>
      )}
    </>
  );
}
