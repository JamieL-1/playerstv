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
    let heardEvent = false;
    const unlisten = events.onStatus((s) => {
      heardEvent = true;
      if (alive) setSnapshot(s);
    });
    // The backend only sends changes, so listen before fetching: a change
    // between the two would otherwise be missed until the next one. An
    // event that arrives first is newer than the fetch's answer.
    unlisten
      .catch(() => {})
      .then(() => backend.getStatus())
      .then((s) => alive && !heardEvent && setSnapshot(s));
    return () => {
      alive = false;
      unlisten.then((f) => f());
    };
  }, []);
  return snapshot;
}

/** Current Unix time, re-rendering every second while `ticking` (for REC timers). */
export function useNow(ticking: boolean = true): number {
  const [now, setNow] = useState(nowSeconds);
  useEffect(() => {
    if (!ticking) return;
    const update = () => setNow(nowSeconds());
    // Straight away too: it may be long out of date if it wasn't ticking.
    const first = setTimeout(update, 0);
    const id = setInterval(update, 1000);
    return () => {
      clearTimeout(first);
      clearInterval(id);
    };
  }, [ticking]);
  return now;
}
