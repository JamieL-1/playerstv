import { Film, LayoutGrid, Radio } from "lucide-react";
import { NavLink, Outlet, useParams, useSearchParams } from "react-router";
import type { GameProfile, Recording, Snapshot } from "@/api/types";
import { ScrollArea } from "@/components/ui/scroll-area";
import { useConfig } from "@/hooks/useConfig";
import { useLibrary } from "@/hooks/useLibrary";
import { useStatus } from "@/hooks/useStatus";
import { countKinds, gameFolder } from "@/lib/library";
import { cn } from "@/lib/utils";
import { RecordingPanel } from "./RecordingPanel";
import { UpdateBanner } from "./UpdateBanner";
import { ServerRail } from "./ServerRail";

/** Discord layout: server rail | channel column (+ recording panel) | page. */
export function AppShell() {
  const { data: config } = useConfig();
  const snapshot = useStatus();
  const { data: recordings } = useLibrary();
  const games = config?.games ?? [];
  return (
    <div className="flex h-screen">
      <ServerRail games={games} snapshot={snapshot} />
      <ChannelColumn games={games} recordings={recordings} snapshot={snapshot} />
      <main className="flex min-w-0 flex-1 flex-col bg-background">
        <Outlet />
      </main>
    </div>
  );
}

const KINDS = [
  { kind: null, label: "All recordings", icon: LayoutGrid, count: "all" },
  { kind: "clips", label: "Clips", icon: Film, count: "clips" },
  { kind: "sessions", label: "Sessions", icon: Radio, count: "sessions" },
] as const;

function ChannelColumn(props: {
  games: GameProfile[];
  recordings: Recording[] | undefined;
  snapshot: Snapshot | null | undefined;
}) {
  const { games, recordings, snapshot } = props;
  const { gameId } = useParams();
  const [params] = useSearchParams();
  const game = games.find((g) => g.id === gameId);
  const current = params.get("kind");
  const counts = recordings && countKinds(recordings, game ? gameFolder(game) : null);
  const base = gameId ? `/game/${encodeURIComponent(gameId)}` : "/";
  return (
    <aside className="flex w-60 shrink-0 flex-col bg-sidebar">
      <header className="flex h-12 items-center border-b border-border px-4 font-semibold shadow-sm">
        {game?.name ?? "All games"}
      </header>
      <ScrollArea className="flex-1">
        <div className="space-y-0.5 p-2">
          {KINDS.map(({ kind, label, icon: Icon, count }) => (
            <NavLink
              key={label}
              to={kind ? `${base}?kind=${kind}` : base}
              className={cn(
                "flex items-center gap-2 rounded-md px-2 py-1.5 text-[15px] font-medium text-muted-foreground transition-colors hover:bg-accent hover:text-foreground",
                current === kind && "bg-selected text-foreground",
              )}
            >
              <Icon className="size-5 opacity-80" />
              <span className="flex-1">{label}</span>
              {counts && <span className="text-xs tabular-nums text-muted-foreground">{counts[count]}</span>}
            </NavLink>
          ))}
        </div>
      </ScrollArea>
      <UpdateBanner snapshot={snapshot} />
      <RecordingPanel snapshot={snapshot} />
    </aside>
  );
}
