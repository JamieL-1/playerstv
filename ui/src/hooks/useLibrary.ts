import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect } from "react";
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
