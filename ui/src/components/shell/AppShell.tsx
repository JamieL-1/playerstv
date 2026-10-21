import { Film, LayoutGrid, Radio } from "lucide-react";
import { NavLink, Outlet, useParams, useSearchParams } from "react-router";
import type { GameProfile, Snapshot } from "@/api/types";
import { ScrollArea } from "@/components/ui/scroll-area";
import { useConfig } from "@/hooks/useConfig";
import { useStatus } from "@/hooks/useStatus";
import { cn } from "@/lib/utils";
import { RecordingPanel } from "./RecordingPanel";
import { ServerRail } from "./ServerRail";

/** Discord layout: server rail | channel column (+ recording panel) | page. */
export function AppShell() {
  const { data: config } = useConfig();
  const snapshot = useStatus();
  const games = config?.games ?? [];
  return (
    <div className="flex h-screen">
      <ServerRail games={games} snapshot={snapshot} />
      <ChannelColumn games={games} snapshot={snapshot} />
      <main className="flex min-w-0 flex-1 flex-col bg-background">
        <Outlet />
      </main>
    </div>
  );
}

const KINDS = [
  { kind: null, label: "All recordings", icon: LayoutGrid },
  { kind: "clips", label: "Clips", icon: Film },
  { kind: "sessions", label: "Sessions", icon: Radio },
] as const;

function ChannelColumn({ games, snapshot }: { games: GameProfile[]; snapshot: Snapshot | null | undefined }) {
  const { gameId } = useParams();
  const [params] = useSearchParams();
  const game = games.find((g) => g.id === gameId);
  const current = params.get("kind");
  const base = gameId ? `/game/${encodeURIComponent(gameId)}` : "/";
  return (
    <aside className="flex w-60 shrink-0 flex-col bg-sidebar">
      <header className="flex h-12 items-center border-b border-border px-4 font-semibold shadow-sm">
        {game?.name ?? "All games"}
      </header>
      <ScrollArea className="flex-1">
        <div className="space-y-0.5 p-2">
          {KINDS.map(({ kind, label, icon: Icon }) => (
            <NavLink
              key={label}
              to={kind ? `${base}?kind=${kind}` : base}
              className={cn(
                "flex items-center gap-2 rounded-md px-2 py-1.5 text-[15px] font-medium text-muted-foreground transition-colors hover:bg-accent hover:text-foreground",
                current === kind && "bg-selected text-foreground",
              )}
            >
              <Icon className="size-5 opacity-80" />
              {label}
            </NavLink>
          ))}
        </div>
      </ScrollArea>
      <RecordingPanel snapshot={snapshot} />
    </aside>
  );
}
