import { convertFileSrc } from "@tauri-apps/api/core";
import {
  Clapperboard,
  ExternalLink,
  Film,
  FolderOpen,
  LayoutGrid,
  MoreHorizontal,
  Play,
  Radio,
  Scissors,
  Search,
  SearchX,
  Trash2,
} from "lucide-react";
import { useDeferredValue, useMemo, useState } from "react";
import { useNavigate, useParams, useSearchParams } from "react-router";
import { toast } from "sonner";
import { backend } from "@/api/backend";
import type { Config, Recording } from "@/api/types";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import { Button } from "@/components/ui/button";
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuTrigger,
} from "@/components/ui/context-menu";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Input } from "@/components/ui/input";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { useConfig } from "@/hooks/useConfig";
import { useLibrary } from "@/hooks/useLibrary";
import { formatElapsed, formatSize, nominalFps } from "@/lib/format";
import {
  canPlay,
  filterRecordings,
  formatWhen,
  gameFolder,
  gameName,
  parseKind,
  SORT_LABELS,
  sortRecordings,
  type KindFilter,
  type SortOrder,
} from "@/lib/library";
import { cn } from "@/lib/utils";

const HEADINGS: Record<NonNullable<KindFilter> | "all", { label: string; icon: typeof Film }> = {
  all: { label: "All recordings", icon: LayoutGrid },
  clips: { label: "Clips", icon: Film },
  sessions: { label: "Sessions", icon: Radio },
};

/** The clip/session grid for every game, or one game (`/game/:gameId`). */
export function LibraryPage() {
  const { gameId } = useParams();
  const [params] = useSearchParams();
  const kind = parseKind(params.get("kind"));
  const { data: config } = useConfig();
  const library = useLibrary();
  const [search, setSearch] = useState("");
  const [sort, setSort] = useState<SortOrder>("newest");
  const [toDelete, setToDelete] = useState<Recording | null>(null);
  const deferredSearch = useDeferredValue(search);

  const games = config?.games ?? [];
  const game = games.find((g) => g.id === gameId);
  const folder = game ? gameFolder(game) : null;
  const all = useMemo(() => library.data ?? [], [library.data]);
  const shown = useMemo(
    () => sortRecordings(filterRecordings(all, { folder, kind, search: deferredSearch }), sort),
    [all, folder, kind, deferredSearch, sort],
  );
  const heading = HEADINGS[kind ?? "all"];
  const actions = useActions(setToDelete);

  let body: React.ReactNode;
  if (library.isPending) {
    body = (
      <Grid>
        {Array.from({ length: 8 }, (_, i) => (
          <SkeletonCard key={i} />
        ))}
      </Grid>
    );
  } else if (library.isError) {
    body = <Empty icon={SearchX} title="Couldn't read the recordings folder" text={String(library.error)} />;
  } else if (filterRecordings(all, { folder, kind, search: "" }).length === 0) {
    body = <NothingRecorded config={config} kind={kind} gameName={game?.name} />;
  } else if (shown.length === 0) {
    body = (
      <Empty icon={SearchX} title="Nothing matches your search" text={`No recordings with “${search}” in their name.`}>
        <Button variant="secondary" onClick={() => setSearch("")}>
          Clear search
        </Button>
      </Empty>
    );
  } else {
    body = (
      <Grid>
        {shown.map((r) => (
          <RecordingCard key={r.path} rec={r} game={folder ? null : gameName(r, games)} actions={actions} />
        ))}
      </Grid>
    );
  }

  return (
    <>
      <header className="flex h-12 shrink-0 items-center gap-2 border-b border-border px-4 shadow-sm">
        <heading.icon className="size-5 text-muted-foreground" />
        <h1 className="font-semibold">{heading.label}</h1>
        {library.data && (
          <span className="text-sm text-muted-foreground">
            {shown.length} {shown.length === 1 ? "recording" : "recordings"}
          </span>
        )}
        <div className="flex-1" />
        <div className="relative">
          <Search className="pointer-events-none absolute left-2 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" />
          <Input
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            onKeyDown={(e) => e.key === "Escape" && setSearch("")}
            placeholder="Search"
            aria-label="Search recordings"
            className="h-8 w-52 bg-rail pl-8"
          />
        </div>
        <Select value={sort} onValueChange={(v) => setSort(v as SortOrder)}>
          <SelectTrigger size="sm" className="w-40" aria-label="Sort">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {(Object.keys(SORT_LABELS) as SortOrder[]).map((o) => (
              <SelectItem key={o} value={o}>
                {SORT_LABELS[o]}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </header>
      <ScrollArea className="min-h-0 flex-1">{body}</ScrollArea>
      <DeleteDialog rec={toDelete} onClose={() => setToDelete(null)} />
    </>
  );
}

function Grid({ children }: { children: React.ReactNode }) {
  return <div className="grid grid-cols-[repeat(auto-fill,minmax(240px,1fr))] gap-4 p-4">{children}</div>;
}

interface Action {
  id: string;
  label: string;
  icon: typeof Play;
  destructive?: boolean;
  separatorBefore?: boolean;
  run: (rec: Recording) => void;
}

function useActions(confirmDelete: (rec: Recording) => void): Action[] {
  const navigate = useNavigate();
  const fail = (e: unknown) => toast.error(String(e));
  return [
    { id: "play", label: "Play", icon: Play, run: (r) => play(r, navigate) },
    {
      id: "trim",
      label: "Trim",
      icon: Scissors,
      run: (r) => navigate(`/watch?path=${encodeURIComponent(r.path)}&trim=1`),
    },
    {
      id: "explorer",
      label: "Show in Explorer",
      icon: FolderOpen,
      separatorBefore: true,
      run: (r) => backend.showInExplorer(r.path).catch(fail),
    },
    {
      id: "external",
      label: "Open in default player",
      icon: ExternalLink,
      run: (r) => backend.openExternal(r.path).catch(fail),
    },
    { id: "delete", label: "Delete", icon: Trash2, destructive: true, separatorBefore: true, run: confirmDelete },
  ];
}

function play(rec: Recording, navigate: ReturnType<typeof useNavigate>) {
  // The player shows an "open in default player" card for files it can't
  // decode, so going there is right either way.
  navigate(`/watch?path=${encodeURIComponent(rec.path)}`);
}

function RecordingCard({ rec, game, actions }: { rec: Recording; game: string | null; actions: Action[] }) {
  const navigate = useNavigate();
  const [menuOpen, setMenuOpen] = useState(false);
  const duration = rec.info?.duration_s;
  const video = rec.info?.video;
  const writing = rec.info === null;
  const details = [game, formatSize(rec.size), video && `${video.height}p${nominalFps(video.fps)}`].filter(Boolean);

  return (
    <ContextMenu>
      <ContextMenuTrigger asChild>
        <div
          role="button"
          tabIndex={0}
          aria-label={`Play ${rec.name}`}
          onClick={() => play(rec, navigate)}
          onKeyDown={(e) => {
            if (e.key === "Enter" || e.key === " ") {
              e.preventDefault();
              play(rec, navigate);
            }
          }}
          className={cn(
            "group cursor-pointer overflow-hidden rounded-lg bg-card outline-none ring-1 ring-border transition hover:ring-primary/60 focus-visible:ring-2 focus-visible:ring-primary",
            menuOpen && "ring-primary/60",
          )}
        >
          <div className="relative aspect-video bg-muted">
            {rec.thumb ? (
              <img src={convertFileSrc(rec.thumb)} alt="" loading="lazy" className="size-full object-cover" />
            ) : (
              <div className="flex size-full items-center justify-center text-muted-foreground">
                <Film className={cn("size-8 opacity-40", !writing && "animate-pulse")} />
              </div>
            )}
            <div className="absolute inset-0 flex items-center justify-center bg-black/35 opacity-0 transition-opacity group-hover:opacity-100">
              <span className="flex size-12 items-center justify-center rounded-full bg-black/60 text-white">
                <Play className="size-6 translate-x-px fill-current" />
              </span>
            </div>
            <div className="absolute left-2 top-2 flex gap-1">
              {rec.kind === "session" && <Tag>Session</Tag>}
              {writing && <Tag>Saving…</Tag>}
              {!writing && !canPlay(rec) && <Tag title="Plays in your default player">External</Tag>}
            </div>
            {duration !== undefined && (
              <span className="absolute bottom-2 right-2 rounded bg-black/75 px-1.5 py-0.5 text-xs font-semibold tabular-nums text-white">
                {formatElapsed(duration)}
              </span>
            )}
          </div>
          <div className="flex items-start gap-1 p-3 pr-1.5">
            <div className="min-w-0 flex-1">
              <div className="truncate text-[15px] font-semibold">{formatWhen(rec.modified)}</div>
              <div className="truncate text-xs text-muted-foreground">{details.join(" · ")}</div>
            </div>
            <DropdownMenu open={menuOpen} onOpenChange={setMenuOpen}>
              <DropdownMenuTrigger asChild>
                <Button
                  size="icon"
                  variant="ghost"
                  aria-label="More"
                  className="size-7 shrink-0 opacity-0 group-hover:opacity-100 focus-visible:opacity-100 data-[state=open]:opacity-100"
                  onClick={(e) => e.stopPropagation()}
                  onKeyDown={(e) => e.stopPropagation()}
                >
                  <MoreHorizontal />
                </Button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end" onClick={(e) => e.stopPropagation()}>
                {actions.map((a) => [
                  a.separatorBefore && <DropdownMenuSeparator key={`${a.id}-sep`} />,
                  <DropdownMenuItem
                    key={a.id}
                    variant={a.destructive ? "destructive" : "default"}
                    onSelect={() => a.run(rec)}
                  >
                    <a.icon />
                    {a.label}
                  </DropdownMenuItem>,
                ])}
              </DropdownMenuContent>
            </DropdownMenu>
          </div>
        </div>
      </ContextMenuTrigger>
      <ContextMenuContent>
        {actions.map((a) => [
          a.separatorBefore && <ContextMenuSeparator key={`${a.id}-sep`} />,
          <ContextMenuItem key={a.id} variant={a.destructive ? "destructive" : "default"} onSelect={() => a.run(rec)}>
            <a.icon />
            {a.label}
          </ContextMenuItem>,
        ])}
      </ContextMenuContent>
    </ContextMenu>
  );
}

function Tag({ children, title }: { children: React.ReactNode; title?: string }) {
  return (
    <span
      title={title}
      className="rounded bg-black/70 px-1.5 py-0.5 text-[11px] font-semibold uppercase tracking-wide text-white"
    >
      {children}
    </span>
  );
}

function SkeletonCard() {
  return (
    <div className="overflow-hidden rounded-lg bg-card ring-1 ring-border">
      <div className="aspect-video animate-pulse bg-muted" />
      <div className="space-y-2 p-3">
        <div className="h-4 w-2/3 animate-pulse rounded bg-muted" />
        <div className="h-3 w-1/2 animate-pulse rounded bg-muted" />
      </div>
    </div>
  );
}

function Empty(props: { icon: typeof Film; title: string; text: string; children?: React.ReactNode }) {
  const Icon = props.icon;
  return (
    <div className="flex flex-col items-center justify-center gap-3 px-6 py-24 text-center text-muted-foreground">
      <Icon className="size-12 opacity-60" />
      <div className="text-lg font-semibold text-foreground">{props.title}</div>
      <div className="max-w-md text-sm">{props.text}</div>
      {props.children}
    </div>
  );
}

function NothingRecorded({ config, kind, gameName }: { config?: Config; kind: KindFilter; gameName?: string }) {
  const what = kind === "sessions" ? "sessions" : kind === "clips" ? "clips" : "recordings";
  const where = gameName ? ` of ${gameName}` : "";
  const text =
    kind === "sessions"
      ? `Press ${config?.session_hotkey ?? "the session hotkey"} in a game to record a full session.`
      : `Press ${config?.clip_hotkey ?? "the clip hotkey"} in a game to save the last ${config?.buffer_seconds ?? 30} seconds.`;
  return (
    <Empty icon={Clapperboard} title={`No ${what}${where} yet`} text={text}>
      {config && (
        <Button variant="secondary" onClick={() => backend.openOutputFolder().catch((e) => toast.error(String(e)))}>
          <FolderOpen /> Open recordings folder
        </Button>
      )}
    </Empty>
  );
}

function DeleteDialog({ rec, onClose }: { rec: Recording | null; onClose: () => void }) {
  return (
    <AlertDialog open={rec !== null} onOpenChange={(open) => !open && onClose()}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>Delete {rec?.kind === "session" ? "session" : "clip"}?</AlertDialogTitle>
          <AlertDialogDescription>
            <span className="font-medium text-foreground">{rec?.name}</span> will be moved to the Recycle Bin.
          </AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel>Cancel</AlertDialogCancel>
          <AlertDialogAction
            variant="destructive"
            onClick={() => {
              if (!rec) return;
              toast.promise(backend.deleteRecording(rec.path), {
                loading: "Deleting…",
                success: "Moved to the Recycle Bin",
                error: (e) => String(e),
              });
            }}
          >
            Delete
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}
