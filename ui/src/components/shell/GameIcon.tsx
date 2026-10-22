import type { GameProfile } from "@/api/types";
import { gameAvatar } from "@/lib/games";
import { cn } from "@/lib/utils";

/** A game's exe icon, or its initials on a colour until the icon is known. */
export function GameIcon({ game, icon, className }: { game: GameProfile; icon?: string; className?: string }) {
  if (icon) {
    return <img src={icon} alt="" draggable={false} className={cn("size-8 object-contain", className)} />;
  }
  return <span className={cn("text-sm font-semibold", className)}>{gameAvatar(game).initials}</span>;
}
