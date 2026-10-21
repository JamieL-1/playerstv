import { Plus, Trash2 } from "lucide-react";
import { BUILTIN_GAME_IDS, type GameProfile } from "@/api/types";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import { gameAvatar } from "@/lib/games";
import { GroupLabel, SectionTitle } from "./controls";
import { useSettings } from "./SettingsContext";

const NOTES: Record<string, string> = {
  runelite: "Old School RuneScape via RuneLite",
  "league-of-legends": "Records matches only, not the client or lobby",
  cs2: "If recordings are black, set Display Mode to Fullscreen Windowed",
};

const isBuiltin = (g: GameProfile) => (BUILTIN_GAME_IDS as readonly string[]).includes(g.id);

function uniqueCustomId(name: string, used: Set<string>): string {
  const base = `custom-${name.toLowerCase().replace(/[^a-z0-9_-]/g, "_") || "game"}`;
  let id = base;
  for (let n = 2; used.has(id); n++) id = `${base}-${n}`;
  return id;
}

export function GamesSection() {
  const { draft, update, setGameEnabled } = useSettings();
  const builtins = draft.games.filter(isBuiltin);
  const customs = draft.games.filter((g) => !isBuiltin(g));
  const editCustom = (id: string, patch: Partial<GameProfile>) =>
    update((c) => ({ ...c, games: c.games.map((g) => (g.id === id ? { ...g, ...patch } : g)) }));

  return (
    <>
      <SectionTitle
        title="Games"
        description="Recording starts automatically while an enabled game is running. Switches apply immediately."
      />
      <div className="space-y-2">
        {builtins.map((g) => (
          <GameCard key={g.id} game={g} note={NOTES[g.id]} onToggle={(v) => setGameEnabled(g.id, v)} />
        ))}
      </div>

      <GroupLabel>Custom games</GroupLabel>
      <p className="mb-3 text-sm text-muted-foreground">
        Detected when the window title matches the pattern and belongs to one of the executables. Click Save to apply
        changes here.
      </p>
      <div className="space-y-3">
        {customs.map((g) => (
          <div key={g.id} className="space-y-3 rounded-lg bg-card p-4">
            <div className="flex items-center gap-3">
              <Switch checked={g.enabled} onCheckedChange={(v) => setGameEnabled(g.id, v)} />
              <Input value={g.name} onChange={(e) => editCustom(g.id, { name: e.target.value })} className="flex-1" />
              <Button
                variant="ghost"
                size="icon"
                aria-label={`Remove ${g.name}`}
                onClick={() => update((c) => ({ ...c, games: c.games.filter((x) => x.id !== g.id) }))}
              >
                <Trash2 />
              </Button>
            </div>
            <label className="block text-xs font-semibold uppercase text-muted-foreground">
              Executables (comma-separated)
              <Input
                className="mt-1 font-mono normal-case"
                value={g.exe_names.join(", ")}
                onChange={(e) =>
                  editCustom(g.id, {
                    exe_names: e.target.value
                      .split(",")
                      .map((s) => s.trim())
                      .filter(Boolean),
                  })
                }
              />
            </label>
            <label className="block text-xs font-semibold uppercase text-muted-foreground">
              Window title pattern (regex)
              <Input
                className="mt-1 font-mono normal-case"
                value={g.window_title_regex}
                onChange={(e) => editCustom(g.id, { window_title_regex: e.target.value })}
              />
            </label>
          </div>
        ))}
      </div>
      <Button
        variant="secondary"
        className="mt-3"
        onClick={() =>
          update((c) => ({
            ...c,
            games: [
              ...c.games,
              {
                id: uniqueCustomId("New game", new Set(c.games.map((g) => g.id))),
                name: "New game",
                exe_names: [],
                window_title_regex: ".*",
                enabled: true,
              },
            ],
          }))
        }
      >
        <Plus /> Add custom game
      </Button>
    </>
  );
}

function GameCard({ game, note, onToggle }: { game: GameProfile; note?: string; onToggle: (v: boolean) => void }) {
  const avatar = gameAvatar(game);
  return (
    <div className="flex items-center gap-4 rounded-lg bg-card p-3">
      <span
        className="flex size-10 shrink-0 items-center justify-center rounded-2xl text-sm font-semibold text-white"
        style={{ background: avatar.colour }}
      >
        {avatar.initials}
      </span>
      <div className="min-w-0 flex-1">
        <div className="font-medium">{game.name}</div>
        {note && <div className="truncate text-sm text-muted-foreground">{note}</div>}
      </div>
      <Switch checked={game.enabled} onCheckedChange={onToggle} aria-label={`Record ${game.name}`} />
    </div>
  );
}
