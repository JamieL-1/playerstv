import { useEffect, useState } from "react";
import { backend, events } from "@/api/backend";
import type { Snapshot } from "@/api/types";
import { nowSeconds } from "@/lib/format";

/**
 * Live tray status. `undefined` while the first answer is pending, `null`
 * when the tray app isn't running.
 */
export function useStatus(): Snapshot | null | undefined {
  const [snapshot, setSnapshot] = useState<Snapshot | null | undefined>(undefined);
  useEffect(() => {
    let alive = true;
    backend.getStatus().then((s) => alive && setSnapshot(s));
    const unlisten = events.onStatus((s) => alive && setSnapshot(s));
    return () => {
      alive = false;
      unlisten.then((f) => f());
    };
  }, []);
  return snapshot;
}

/** Current Unix time, re-rendering every second (for REC timers). */
export function useNow(): number {
  const [now, setNow] = useState(nowSeconds);
  useEffect(() => {
    const id = setInterval(() => setNow(nowSeconds()), 1000);
    return () => clearInterval(id);
  }, []);
  return now;
}
