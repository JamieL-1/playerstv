import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect } from "react";
import { backend, events } from "@/api/backend";
import type { Config } from "@/api/types";

export const CONFIG_KEY = ["config"] as const;

/** The saved config, kept current when config.toml changes on disk. */
export function useConfig() {
  const client = useQueryClient();
  useEffect(() => {
    const unlisten = events.onConfigChanged((cfg) => client.setQueryData<Config>(CONFIG_KEY, cfg));
    return () => {
      unlisten.then((f) => f());
    };
  }, [client]);
  return useQuery({ queryKey: CONFIG_KEY, queryFn: backend.getConfig, staleTime: Infinity });
}
