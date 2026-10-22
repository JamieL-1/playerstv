import { Clapperboard, Plus, Settings } from "lucide-react";
import { NavLink, useLocation, useNavigate } from "react-router";
import { toast } from "sonner";
import { backend } from "@/api/backend";
import type { GameProfile, Snapshot } from "@/api/types";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { useGameIcons } from "@/hooks/useGameIcons";
import { gameAvatar } from "@/lib/games";
import { cn } from "@/lib/utils";
import { GameIcon } from "./GameIcon";

/** Discord's left "server" rail: All recordings, one icon per game, add a game, settings. */
export function ServerRail({ games, snapshot }: { games: GameProfile[]; snapshot: Snapshot | null | undefined }) {
  const recordingGame = snapshot?.status.state === "recording" ? snapshot.status.game : null;
  const { pathname } = useLocation();
  const icons = useGameIcons(games);
  return (
    <nav className="flex w-[72px] shrink-0 flex-col items-center gap-2 bg-rail py-3" aria-label="Games">
      <RailItem to="/" label="All recordings" active={pathname === "/"}>
        <Clapperboard className="size-6" />
      </RailItem>
      <div className="h-0.5 w-8 rounded-full bg-border" />
      {games.map((g) => {
        const icon = icons[g.id];
        const to = `/game/${encodeURIComponent(g.id)}`;
        return (
          <RailItem
            key={g.id}
            to={to}
            label={g.enabled ? g.name : `${g.name} (not recorded)`}
            active={pathname === to}
            colour={icon ? "var(--background)" : gameAvatar(g).colour}
            dimmed={!g.enabled}
            recording={recordingGame === g.name}
          >
            <GameIcon game={g} icon={icon} />
          </RailItem>
        );
      })}
      <AddGame />
      <div className="flex-1" />
      <RailItem to="/settings/recording" label="Settings" active={pathname.startsWith("/settings")}>
        <Settings className="size-6" />
      </RailItem>
    </nav>
  );
}

/** An empty slot with a "+": pick a game's exe to start recording it. */
function AddGame() {
  const navigate = useNavigate();
  const add = async () => {
    try {
      if (await backend.addGame()) navigate("/settings/games");
    } catch (e) {
      toast.error(`Couldn't add the game: ${String(e)}`);
    }
  };
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <button
          type="button"
          aria-label="Add a game"
          onClick={add}
          className="flex size-12 items-center justify-center rounded-[24px] border-2 border-dashed border-muted-foreground/40 text-[#23a55a] transition-all duration-150 hover:rounded-2xl hover:border-solid hover:border-[#23a55a] hover:bg-[#23a55a] hover:text-white"
        >
          <Plus className="size-6" />
        </button>
      </TooltipTrigger>
      <TooltipContent side="right">Add a game</TooltipContent>
    </Tooltip>
  );
}

function RailItem(props: {
  to: string;
  label: string;
  active: boolean;
  colour?: string;
  dimmed?: boolean;
  recording?: boolean;
  children: React.ReactNode;
}) {
  const { to, label, active, colour, dimmed, recording, children } = props;
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <div className="group relative flex w-full justify-center">
          {/* Discord's selection pill on the left edge. */}
          <span
            className={cn(
              "absolute left-0 top-1/2 w-1 -translate-y-1/2 rounded-r-full bg-foreground transition-all",
              active ? "h-10" : "h-0 group-hover:h-5",
            )}
          />
          <NavLink
            to={to}
            aria-label={label}
            className={cn(
              "relative flex size-12 items-center justify-center overflow-visible text-foreground transition-all duration-150",
              active ? "rounded-2xl" : "rounded-[24px] hover:rounded-2xl",
              dimmed && "opacity-40",
            )}
            style={{
              background: active || colour ? (colour ?? "var(--primary)") : "var(--background)",
              color: active || colour ? "#fff" : undefined,
            }}
          >
            {children}
            {recording && (
              <span
                className="absolute -bottom-0.5 -right-0.5 size-4 animate-pulse rounded-full border-[3px] border-rail bg-red-500"
                aria-label="recording"
              />
            )}
          </NavLink>
        </div>
      </TooltipTrigger>
      <TooltipContent side="right">{label}</TooltipContent>
    </Tooltip>
  );
}
