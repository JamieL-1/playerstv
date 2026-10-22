import { useQuery } from "@tanstack/react-query";
import { backend } from "@/api/backend";
import type { GameProfile } from "@/api/types";

/**
 * Game id → icon (a data URL) for games whose exe has been found. Refetched
 * when the game list changes, and now and then while some are missing, since
 * a game's exe is found once it has been run.
 */
export function useGameIcons(games: GameProfile[]): Record<string, string> {
  const ids = games.map((g) => g.id);
  const { data } = useQuery({
    queryKey: ["game-icons", ids],
    queryFn: backend.gameIcons,
    staleTime: Infinity,
    placeholderData: (previous) => previous,
    refetchInterval: (q) => (ids.every((id) => q.state.data?.[id]) ? false : 60_000),
  });
  return data ?? {};
}
