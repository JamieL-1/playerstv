import { Clapperboard } from "lucide-react";

// Milestone 3 replaces this with the clip/session grid.
export function LibraryPage() {
  return (
    <div className="flex flex-1 flex-col items-center justify-center gap-3 text-muted-foreground">
      <Clapperboard className="size-12 opacity-60" />
      <div className="text-lg font-semibold text-foreground">Your clips will show up here</div>
      <div className="text-sm">The library is coming in the next milestone.</div>
    </div>
  );
}
