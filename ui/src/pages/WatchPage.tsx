import { useQueryClient } from "@tanstack/react-query";
import { convertFileSrc } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  ArrowLeft,
  ExternalLink,
  FolderOpen,
  Maximize,
  Minimize,
  Pause,
  Pencil,
  Play,
  Scissors,
  SkipBack,
  SkipForward,
  TriangleAlert,
  Volume2,
  VolumeX,
} from "lucide-react";
import { useCallback, useEffect, useEffectEvent, useRef, useState } from "react";
import { useLocation, useNavigate, useSearchParams } from "react-router";
import { toast } from "sonner";
import { backend } from "@/api/backend";
import type { ExportDone, Recording } from "@/api/types";
import { SeekBar } from "@/components/player/SeekBar";
import { NameDialog } from "@/components/library/NameDialog";
import { TrimTimeline } from "@/components/player/TrimTimeline";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Progress } from "@/components/ui/progress";
import { Slider } from "@/components/ui/slider";
import { Toggle } from "@/components/ui/toggle";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { useConfig } from "@/hooks/useConfig";
import { useExport } from "@/hooks/useExport";
import { LIBRARY_KEY, useLibrary, useRename } from "@/hooks/useLibrary";
import { formatElapsed, formatSize, nominalFps } from "@/lib/format";
import { canPlay, defaultClipName, displayName, formatWhen, gameName } from "@/lib/library";
import { clamp, formatPrecise, setIn, setOut, snapToFrame, type Range } from "@/lib/timeline";
import { cn } from "@/lib/utils";

const SPEEDS = [0.25, 0.5, 1, 1.5, 2];
const VOLUME_KEY = "ptv.player.volume";

/** `/watch?path=…[&trim=1]`: the player, with the trim editor below it. */
export function WatchPage() {
  const [params, setParams] = useSearchParams();
  const path = params.get("path") ?? "";
  const trimming = params.get("trim") === "1";
  const library = useLibrary();
  const rec = library.data?.find((r) => r.path === path);
  const back = useBack();

  const setTrimming = useCallback(
    (on: boolean) => setParams(on ? { path, trim: "1" } : { path }, { replace: true }),
    [path, setParams],
  );

  if (!rec) {
    return (
      <div className="flex h-screen flex-col bg-background">
        <Header title={library.isPending ? "" : "Recording not found"} onBack={back} />
        {!library.isPending && (
          <Notice
            title="This recording no longer exists"
            text="It may have been deleted, moved or renamed."
            action={<Button onClick={back}>Back to library</Button>}
          />
        )}
      </div>
    );
  }
  // Keyed by path: switching recordings starts from a clean player.
  return <Watch key={rec.path} rec={rec} trimming={trimming} setTrimming={setTrimming} onBack={back} />;
}

function useBack() {
  const navigate = useNavigate();
  const location = useLocation();
  // Back to wherever the library was (filters included), or home.
  return useCallback(() => (location.key !== "default" ? navigate(-1) : navigate("/")), [location.key, navigate]);
}

function Watch(props: { rec: Recording; trimming: boolean; setTrimming: (on: boolean) => void; onBack: () => void }) {
  const { rec, trimming, setTrimming, onBack } = props;
  const { data: config } = useConfig();
  const client = useQueryClient();
  const navigate = useNavigate();
  const video = useRef<HTMLVideoElement>(null);
  const [failed, setFailed] = useState(false);
  const playable = canPlay(rec) && !failed;
  const fps = nominalFps(rec.info?.video?.fps ?? 0);

  const [time, setTime] = useState(0);
  const [duration, setDuration] = useState(rec.info?.duration_s ?? 0);
  const [playing, setPlaying] = useState(false);
  const [rate, setRate] = useState(1);
  const [{ volume, muted }, setVolumeState] = useState(loadVolume);
  const [fullscreen, setFullscreen] = useState(false);
  const [range, setRange] = useState<Range>({ start: 0, end: rec.info?.duration_s ?? 0 });

  const exportDone = (d: ExportDone) => {
    if (d.state === "saved") {
      client.invalidateQueries({ queryKey: LIBRARY_KEY });
      toast.success("Clip saved", {
        description: d.path.split(/[\\/]/).pop(),
        action: { label: "Open", onClick: () => navigate(`/watch?path=${encodeURIComponent(d.path)}`) },
        cancel: { label: "Show in Explorer", onClick: () => backend.showInExplorer(d.path).catch(() => {}) },
      });
    } else if (d.state === "failed") {
      toast.error(d.error);
    } else {
      toast("Export cancelled");
    }
  };
  const { exporting, start: startExport, cancel: cancelExport } = useExport(exportDone);
  // The default name of the clip being named before export, or null.
  const [newClipName, setNewClipName] = useState<string | null>(null);
  const [renaming, setRenaming] = useState(false);
  const rename = useRename();

  // Smooth playhead while playing (timeupdate only fires ~4×/s); in trim
  // mode, playback loops inside the selection.
  const onFrame = useEffectEvent(() => {
    const v = video.current;
    if (!v) return;
    if (trimming && v.currentTime >= range.end) v.currentTime = range.start;
    setTime(v.currentTime);
  });
  useEffect(() => {
    if (!playing) return;
    let frame = 0;
    const tick = () => {
      onFrame();
      frame = requestAnimationFrame(tick);
    };
    frame = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(frame);
  }, [playing]);

  useEffect(() => {
    const v = video.current;
    if (!v) return;
    v.volume = volume;
    v.muted = muted;
    try {
      localStorage.setItem(VOLUME_KEY, JSON.stringify({ volume, muted }));
    } catch {
      // Storage unavailable: the volume just isn't remembered.
    }
  }, [volume, muted]);

  const seek = useCallback(
    (t: number) => {
      const v = video.current;
      if (!v) return;
      v.currentTime = clamp(t, 0, duration || v.duration || 0);
      setTime(v.currentTime);
    },
    [duration],
  );

  const togglePlay = useCallback(() => {
    const v = video.current;
    if (!v) return;
    if (v.paused) {
      // Previewing a trim starts at the in point.
      if (trimming && (v.currentTime < range.start || v.currentTime >= range.end - 0.01)) v.currentTime = range.start;
      v.play().catch(() => {});
    } else v.pause();
  }, [trimming, range]);

  const step = useCallback(
    (frames: number) => {
      const v = video.current;
      if (!v) return;
      v.pause();
      seek(snapToFrame(v.currentTime, fps) + frames / (fps || 60));
    },
    [fps, seek],
  );

  const toggleFullscreen = useCallback(async (on?: boolean) => {
    const next = on ?? !(await getCurrentWindow().isFullscreen());
    await getCurrentWindow().setFullscreen(next);
    setFullscreen(next);
  }, []);
  // Leaving the page never leaves the window fullscreen.
  useEffect(() => () => void getCurrentWindow().setFullscreen(false), []);

  const markIn = () => {
    if (!trimming) setTrimming(true);
    setRange((r) => setIn(r, snapToFrame(time, fps), duration));
  };
  const markOut = () => {
    if (!trimming) setTrimming(true);
    setRange((r) => setOut(r, snapToFrame(time, fps), duration));
  };

  // Keyboard: YouTube-style, plus editor keys for trimming.
  const onKey = useEffectEvent((e: KeyboardEvent) => {
    const typing =
      e.target instanceof Element && e.target.closest("input, textarea, [role=menu], [role=listbox], [role=dialog]");
    if (e.ctrlKey || e.altKey || e.metaKey || typing) return;
    const v = video.current;
    const actions: Record<string, () => void> = {
      " ": togglePlay,
      k: togglePlay,
      j: () => seek(time - 10),
      l: () => seek(time + 10),
      arrowleft: () => seek(time - 5),
      arrowright: () => seek(time + 5),
      ",": () => step(-1),
      ".": () => step(1),
      home: () => seek(trimming ? range.start : 0),
      end: () => seek(trimming ? range.end : duration),
      f: () => toggleFullscreen(),
      m: () => setVolumeState((s) => ({ ...s, muted: !s.muted })),
      i: markIn,
      o: markOut,
      escape: () => {
        if (e.defaultPrevented) return;
        if (fullscreen) toggleFullscreen(false);
        else onBack();
      },
    };
    const action = actions[e.key.toLowerCase()];
    if (!action || (!v && e.key !== "Escape")) return;
    if (e.key !== "Escape") e.preventDefault();
    action();
  });
  useEffect(() => {
    const handler = (e: KeyboardEvent) => onKey(e);
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, []);

  // Controls fade out while playing and the mouse is still.
  const [idle, setIdle] = useState(false);
  const idleTimer = useRef<ReturnType<typeof setTimeout>>(undefined);
  const wake = () => {
    setIdle(false);
    clearTimeout(idleTimer.current);
    idleTimer.current = setTimeout(() => setIdle(true), 2500);
  };
  useEffect(() => () => clearTimeout(idleTimer.current), []);
  const showControls = !playing || !idle;

  const tracks = rec.info?.audio_tracks ?? [];
  const game = gameName(rec, config?.games ?? []);
  const length = range.end - range.start;

  return (
    <div className="flex h-screen flex-col bg-background">
      <Header
        title={rec.custom_name ?? formatWhen(rec.modified)}
        subtitle={[rec.custom_name && formatWhen(rec.modified), game, formatSize(rec.size)].filter(Boolean).join(" · ")}
        onBack={onBack}
      >
        {tracks.length > 0 && playable && (
          <Tooltip>
            <TooltipTrigger asChild>
              <span className="flex items-center gap-1.5 rounded-md bg-rail px-2 py-1 text-xs text-muted-foreground">
                <Volume2 className="size-3.5" /> {tracks[0]}
              </span>
            </TooltipTrigger>
            <TooltipContent className="max-w-64">
              Playing the "{tracks[0]}" track.
              {tracks.length > 1 && ` Exported clips keep all ${tracks.length} tracks: ${tracks.join(", ")}.`}
            </TooltipContent>
          </Tooltip>
        )}
        {playable && (
          <Toggle
            size="sm"
            variant="outline"
            pressed={trimming}
            onPressedChange={setTrimming}
            aria-label="Trim"
            className="gap-1.5"
          >
            <Scissors /> Trim
          </Toggle>
        )}
        <HeaderButton label="Rename" onClick={() => setRenaming(true)}>
          <Pencil />
        </HeaderButton>
        <HeaderButton label="Show in Explorer" onClick={() => backend.showInExplorer(rec.path).catch(fail)}>
          <FolderOpen />
        </HeaderButton>
        <HeaderButton label="Open in default player" onClick={() => backend.openExternal(rec.path).catch(fail)}>
          <ExternalLink />
        </HeaderButton>
      </Header>

      <div className="flex min-h-0 flex-1 flex-col gap-4 p-4">
        {!playable ? (
          <Notice
            title="Can't preview this file here"
            text={
              failed
                ? "The player couldn't decode this recording."
                : rec.info === null
                  ? "This recording is still being saved, or couldn't be read."
                  : `It's ${describeVideo(rec)}, which the app's player can't decode.`
            }
            action={
              <Button onClick={() => backend.openExternal(rec.path).catch(fail)}>
                <ExternalLink /> Open in default player
              </Button>
            }
          />
        ) : (
          <div
            className={cn(
              "group/player relative min-h-0 flex-1 overflow-hidden rounded-lg bg-black",
              fullscreen && "fixed inset-0 z-50 rounded-none",
              !showControls && "cursor-none",
            )}
            onMouseMove={wake}
            onMouseLeave={() => setIdle(true)}
          >
            <video
              ref={video}
              src={convertFileSrc(rec.path)}
              preload="auto"
              autoPlay={!trimming}
              className="size-full object-contain"
              onClick={togglePlay}
              onDoubleClick={() => toggleFullscreen()}
              onPlay={() => setPlaying(true)}
              onPause={() => setPlaying(false)}
              onTimeUpdate={(e) => !playing && setTime(e.currentTarget.currentTime)}
              onSeeked={(e) => setTime(e.currentTarget.currentTime)}
              onLoadedMetadata={(e) => {
                const d = e.currentTarget.duration;
                if (Number.isFinite(d) && d > 0) {
                  setDuration(d);
                  setRange((r) => (r.end > 0 && r.end <= d ? r : { start: 0, end: d }));
                }
              }}
              onRateChange={(e) => setRate(e.currentTarget.playbackRate)}
              onError={() => setFailed(true)}
            />
            <div
              className={cn(
                "absolute inset-x-0 bottom-0 bg-gradient-to-t from-black/80 to-transparent px-4 pb-2 pt-10 text-white transition-opacity",
                showControls ? "opacity-100" : "opacity-0",
              )}
            >
              <SeekBar time={time} duration={duration} selection={trimming ? range : null} onSeek={seek} />
              <div className="flex items-center gap-1">
                <ControlButton label={playing ? "Pause (K)" : "Play (K)"} onClick={togglePlay}>
                  {playing ? <Pause className="fill-current" /> : <Play className="fill-current" />}
                </ControlButton>
                <ControlButton label="Previous frame (,)" onClick={() => step(-1)}>
                  <SkipBack />
                </ControlButton>
                <ControlButton label="Next frame (.)" onClick={() => step(1)}>
                  <SkipForward />
                </ControlButton>
                <div className="group/volume flex items-center">
                  <ControlButton
                    label={muted ? "Unmute (M)" : "Mute (M)"}
                    onClick={() => setVolumeState((s) => ({ ...s, muted: !s.muted }))}
                  >
                    {muted || volume === 0 ? <VolumeX /> : <Volume2 />}
                  </ControlButton>
                  <Slider
                    aria-label="Volume"
                    className="w-0 opacity-0 transition-all group-hover/volume:w-20 group-hover/volume:opacity-100 focus-within:w-20 focus-within:opacity-100"
                    min={0}
                    max={1}
                    step={0.01}
                    value={[muted ? 0 : volume]}
                    onValueChange={([v]) => setVolumeState({ volume: v ?? 0, muted: false })}
                  />
                </div>
                <span className="ml-2 text-sm tabular-nums">
                  {formatElapsed(time)} / {formatElapsed(duration)}
                </span>
                <div className="flex-1" />
                <DropdownMenu>
                  <DropdownMenuTrigger asChild>
                    <Button
                      variant="ghost"
                      size="sm"
                      className="h-8 px-2 text-sm tabular-nums text-white hover:bg-white/15 hover:text-white"
                      aria-label="Playback speed"
                    >
                      {rate}×
                    </Button>
                  </DropdownMenuTrigger>
                  <DropdownMenuContent align="end" side="top">
                    <DropdownMenuRadioGroup
                      value={String(rate)}
                      onValueChange={(v) => video.current && (video.current.playbackRate = Number(v))}
                    >
                      {SPEEDS.map((s) => (
                        <DropdownMenuRadioItem key={s} value={String(s)}>
                          {s === 1 ? "Normal" : `${s}×`}
                        </DropdownMenuRadioItem>
                      ))}
                    </DropdownMenuRadioGroup>
                  </DropdownMenuContent>
                </DropdownMenu>
                <ControlButton
                  label={fullscreen ? "Exit full screen (F)" : "Full screen (F)"}
                  onClick={() => toggleFullscreen()}
                >
                  {fullscreen ? <Minimize /> : <Maximize />}
                </ControlButton>
              </div>
            </div>
          </div>
        )}

        {trimming && playable && (
          <div className="shrink-0 space-y-3 rounded-lg bg-card p-4 ring-1 ring-border">
            <TrimTimeline
              time={time}
              duration={duration}
              fps={fps}
              range={range}
              onRangeChange={setRange}
              onSeek={seek}
            />
            <div className="flex flex-wrap items-center gap-2">
              <Button variant="secondary" size="sm" onClick={markIn}>
                Set in <Kbd>I</Kbd>
              </Button>
              <span className="w-24 text-sm tabular-nums">{formatPrecise(range.start)}</span>
              <Button variant="secondary" size="sm" onClick={markOut}>
                Set out <Kbd>O</Kbd>
              </Button>
              <span className="w-24 text-sm tabular-nums">{formatPrecise(range.end)}</span>
              <span className="text-sm text-muted-foreground">
                Length <span className="font-semibold tabular-nums text-foreground">{formatPrecise(length)}</span>
              </span>
              <div className="flex-1" />
              {exporting ? (
                <div className="flex items-center gap-3">
                  <Progress value={exporting.fraction * 100} className="w-40" />
                  <span className="w-10 text-right text-sm tabular-nums">{Math.round(exporting.fraction * 100)}%</span>
                  <Button variant="outline" size="sm" onClick={cancelExport} disabled={exporting.id === null}>
                    Cancel
                  </Button>
                </div>
              ) : (
                <Button
                  size="sm"
                  onClick={() => {
                    video.current?.pause();
                    setNewClipName(defaultClipName(rec.game));
                  }}
                >
                  <Scissors /> Save clip
                </Button>
              )}
            </div>
          </div>
        )}
      </div>

      <NameDialog
        open={newClipName !== null}
        title="Save clip"
        description={`Name the new ${formatPrecise(length)} clip. It's saved with your other ${game} clips.`}
        initial={newClipName ?? ""}
        confirm="Save clip"
        onClose={() => setNewClipName(null)}
        onSubmit={(name) => {
          // Left as suggested: saved exactly as an unnamed clip would be.
          startExport(rec.path, range.start, range.end, name === newClipName ? null : name);
        }}
      />
      <NameDialog
        open={renaming}
        title={`Rename ${rec.kind === "session" ? "session" : "clip"}`}
        description="The file is renamed too, keeping its date and time at the end."
        initial={displayName(rec)}
        confirm="Rename"
        onClose={() => setRenaming(false)}
        onSubmit={async (name) => {
          if (name === displayName(rec)) return;
          const path = await rename(rec, name);
          navigate(`/watch?path=${encodeURIComponent(path)}${trimming ? "&trim=1" : ""}`, { replace: true });
        }}
      />
    </div>
  );
}

function fail(e: unknown) {
  toast.error(String(e));
}

function loadVolume(): { volume: number; muted: boolean } {
  try {
    const saved = JSON.parse(localStorage.getItem(VOLUME_KEY) ?? "null");
    if (typeof saved?.volume === "number") return { volume: clamp(saved.volume, 0, 1), muted: !!saved.muted };
  } catch {
    // Fall through to the default.
  }
  return { volume: 1, muted: false };
}

/** "an H.264 4:4:4 recording", "an HEVC recording". */
function describeVideo(rec: Recording): string {
  const v = rec.info?.video;
  if (!v) return "a recording without video";
  const names: Record<string, string> = { h264: "H.264", hevc: "HEVC", av1: "AV1" };
  const name = names[v.codec] ?? v.codec.toUpperCase();
  return v.playable ? `${name} video` : `${name} video in a 4:4:4 or 4:2:2 format (from an older playerstv version)`;
}

function Header(props: { title: string; subtitle?: string; onBack: () => void; children?: React.ReactNode }) {
  return (
    <header className="flex h-12 shrink-0 items-center gap-2 border-b border-border px-2 shadow-sm">
      <HeaderButton label="Back (Esc)" onClick={props.onBack}>
        <ArrowLeft />
      </HeaderButton>
      <div className="min-w-0">
        <span className="font-semibold">{props.title}</span>
        {props.subtitle && <span className="ml-2 text-sm text-muted-foreground">{props.subtitle}</span>}
      </div>
      <div className="flex-1" />
      <div className="flex items-center gap-1 pr-2">{props.children}</div>
    </header>
  );
}

function HeaderButton(props: { label: string; onClick: () => void; children: React.ReactNode }) {
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <Button size="icon" variant="ghost" aria-label={props.label} onClick={props.onClick} className="size-8">
          {props.children}
        </Button>
      </TooltipTrigger>
      <TooltipContent>{props.label}</TooltipContent>
    </Tooltip>
  );
}

function ControlButton(props: { label: string; onClick: () => void; children: React.ReactNode }) {
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <Button
          size="icon"
          variant="ghost"
          aria-label={props.label}
          onClick={props.onClick}
          className="size-8 text-white hover:bg-white/15 hover:text-white"
        >
          {props.children}
        </Button>
      </TooltipTrigger>
      <TooltipContent>{props.label}</TooltipContent>
    </Tooltip>
  );
}

function Kbd({ children }: { children: React.ReactNode }) {
  return (
    <kbd className="ml-1 rounded border border-border bg-background px-1 font-mono text-[11px] text-muted-foreground">
      {children}
    </kbd>
  );
}

function Notice(props: { title: string; text: string; action?: React.ReactNode }) {
  return (
    <div className="flex flex-1 flex-col items-center justify-center gap-3 rounded-lg bg-card p-8 text-center ring-1 ring-border">
      <TriangleAlert className="size-10 text-amber-400" />
      <div className="text-lg font-semibold">{props.title}</div>
      <div className="max-w-md text-sm text-muted-foreground">{props.text}</div>
      {props.action}
    </div>
  );
}
