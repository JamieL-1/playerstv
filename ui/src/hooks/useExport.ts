import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import { backend, events } from "@/api/backend";
import type { ExportDone } from "@/api/types";

export interface ExportState {
  id: number | null;
  fraction: number;
}

/**
 * One trim export at a time: start it, follow its progress, cancel it.
 * `onDone` gets the backend's final word (saved / failed / cancelled).
 */
export function useExport(onDone: (done: ExportDone) => void) {
  const [state, setState] = useState<ExportState | null>(null);
  const current = useRef<number | null>(null);
  // A short export can finish before export_trim has returned its id.
  const early = useRef(new Map<number, ExportDone>());
  const done = useRef(onDone);
  useLayoutEffect(() => {
    done.current = onDone;
  });

  const finish = useCallback((d: ExportDone) => {
    current.current = null;
    setState(null);
    done.current(d);
  }, []);

  useEffect(() => {
    const unlistenProgress = events.onExportProgress(({ id, fraction }) => {
      if (id === current.current) setState({ id, fraction });
    });
    const unlistenDone = events.onExportDone((d) => {
      if (d.id === current.current) finish(d);
      else early.current.set(d.id, d);
    });
    return () => {
      unlistenProgress.then((f) => f());
      unlistenDone.then((f) => f());
    };
  }, [finish]);

  const start = useCallback(
    async (path: string, startS: number, endS: number, name: string | null) => {
      setState({ id: null, fraction: 0 });
      try {
        const id = await backend.exportTrim(path, startS, endS, name);
        const finished = early.current.get(id);
        early.current.clear();
        if (finished) return finish(finished);
        current.current = id;
        setState((s) => (s ? { ...s, id } : s));
      } catch (e) {
        finish({ state: "failed", id: -1, error: String(e) });
      }
    },
    [finish],
  );

  const cancel = useCallback(() => {
    if (current.current !== null) backend.cancelTrim(current.current);
  }, []);

  return { exporting: state, start, cancel };
}
