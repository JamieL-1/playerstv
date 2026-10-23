import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useCallback, useEffect } from "react";
import { backend, events } from "@/api/backend";
import type { Recording } from "@/api/types";

export const LIBRARY_KEY = ["library"] as const;

/**
 * Every recording, newest first. Refetched when the output folder changes;
 * thumbnails are filled in as the backend finishes them.
 */
export function useLibrary() {
  const client = useQueryClient();
  useEffect(() => {
    const unlistenChanged = events.onLibraryChanged(() => client.invalidateQueries({ queryKey: LIBRARY_KEY }));
    const unlistenThumb = events.onThumbnailReady(({ path, thumb }) =>
      client.setQueryData<Recording[]>(LIBRARY_KEY, (recs) =>
        recs?.map((r) => (r.path === path ? { ...r, thumb } : r)),
      ),
    );
    return () => {
      unlistenChanged.then((f) => f());
      unlistenThumb.then((f) => f());
    };
  }, [client]);
  return useQuery({ queryKey: LIBRARY_KEY, queryFn: backend.listRecordings, staleTime: Infinity });
}

/**
 * Renames a recording, updating the library straight away (so a page
 * showing it can follow the new path); resolves to that path.
 */
export function useRename() {
  const client = useQueryClient();
  return useCallback(
    async (rec: Recording, name: string) => {
      const path = await backend.renameRecording(rec.path, name);
      const file = path.split(/[\\/]/).pop() ?? rec.name;
      client.setQueryData<Recording[]>(LIBRARY_KEY, (recs) =>
        recs?.map((r) => (r.path === rec.path ? { ...r, path, name: file, custom_name: name } : r)),
      );
      // The backend has the last word on custom_name (a default name isn't one).
      void client.invalidateQueries({ queryKey: LIBRARY_KEY });
      return path;
    },
    [client],
  );
}
