import { ArrowLeft, RotateCcw } from "lucide-react";
import { Navigate, useNavigate, useParams } from "react-router";
import {
  LIMITS,
  type AudioOverrides,
  type AudioSourceKind,
  type Codec,
  type GameOverrides,
  type Quality,
  type Resolution,
  type VideoOverrides,
} from "@/api/types";
import { Button } from "@/components/ui/button";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Slider } from "@/components/ui/slider";
import { Switch } from "@/components/ui/switch";
import { useGameIcons } from "@/hooks/useGameIcons";
import { effectiveSettings, formatBuffer, updateOverrides } from "@/lib/overrides";
import { GroupLabel, Hint, SettingRow } from "./controls";
import { GameBadge } from "./GamesSection";
import { CODECS, labelOf, QUALITIES, RESOLUTIONS, usePlayableCodecs } from "./options";
import { useSettings } from "./SettingsContext";

const DEFAULT = "__default__";

const ON_OFF: { value: "on" | "off"; label: string }[] = [
  { value: "on", label: "On" },
  { value: "off", label: "Off" },
];
const toOnOff = (b: boolean | undefined) => (b === undefined ? undefined : b ? "on" : "off");
const fromOnOff = (s: "on" | "off" | undefined) => (s === undefined ? undefined : s === "on");

const MODES: { value: "session" | "clips"; label: string }[] = [
  { value: "clips", label: "Clips only" },
  { value: "session", label: "Clips + full session" },
];

const SOURCES: { value: AudioSourceKind; label: string }[] = [
  { value: "game", label: "Only the game" },
  { value: "desktop", label: "All desktop audio" },
];

/** One game's own recording settings; anything on Default follows the global settings. */
export function GameSettingsPage() {
  const { gameId } = useParams();
  const navigate = useNavigate();
  const { draft, update } = useSettings();
  const icons = useGameIcons(draft.games);
  const playable = usePlayableCodecs();
  const game = draft.games.find((g) => g.id === gameId);
  if (!game) return <Navigate to="/settings/games" replace />;

  const o = game.overrides ?? {};
  const v = o.video ?? {};
  const a = o.audio ?? {};
  const g = draft; // the global settings
  const eff = effectiveSettings(draft, game);
  const edit = (fn: (o: GameOverrides) => GameOverrides) => update((c) => updateOverrides(c, game.id, fn));
  const setVideo = (patch: VideoOverrides) => edit((x) => ({ ...x, video: { ...x.video, ...patch } }));
  const setAudio = (patch: AudioOverrides) => edit((x) => ({ ...x, audio: { ...x.audio, ...patch } }));
  const onOff = (b: boolean) => (b ? "On" : "Off");
  const both = eff.audio.game_audio && eff.audio.mic;
  const [vMin, vMax] = LIMITS.volume;
  const [capMin, capMax] = LIMITS.bitrateCapMbps;

  return (
    <>
      <Button variant="ghost" size="sm" className="-ml-2 mb-3" onClick={() => navigate("/settings/games")}>
        <ArrowLeft /> Games
      </Button>
      <div className="mb-5 flex items-center gap-4">
        <GameBadge game={game} icon={icons[game.id]} />
        <div className="min-w-0 flex-1">
          <h1 className="text-xl font-semibold">{game.name}</h1>
          <p className="mt-1 text-sm text-muted-foreground">
            Settings left on Default follow your global settings, including when you change them later.
          </p>
        </div>
        <Button variant="secondary" disabled={!game.overrides} onClick={() => edit(() => ({}))}>
          <RotateCcw /> Reset to defaults
        </Button>
      </div>

      <GroupLabel>Recording</GroupLabel>
      <SettingRow
        label="Recording mode"
        description="A full session saves everything from when the game opens until it closes."
      >
        <DefaultSelect
          value={o.record_full_session === undefined ? undefined : o.record_full_session ? "session" : "clips"}
          options={MODES}
          defaultLabel={labelOf(MODES, g.record_full_session ? "session" : "clips")}
          onChange={(m) => edit((x) => ({ ...x, record_full_session: m === undefined ? undefined : m === "session" }))}
        />
      </SettingRow>
      <DefaultSliderRow
        label="Replay buffer length"
        value={o.buffer_seconds}
        global={g.buffer_seconds}
        min={LIMITS.bufferSeconds[0]}
        max={LIMITS.bufferSeconds[1]}
        step={5}
        format={(s) => `${s} s`}
        defaultFormat={formatBuffer}
        onChange={(s) => edit((x) => ({ ...x, buffer_seconds: s }))}
      />

      <GroupLabel>Video</GroupLabel>
      <SettingRow label="Resolution">
        <DefaultSelect<Resolution>
          value={v.resolution}
          options={RESOLUTIONS}
          defaultLabel={labelOf(RESOLUTIONS, g.video.resolution)}
          onChange={(resolution) => setVideo({ resolution })}
        />
      </SettingRow>
      <SettingRow label="Frame rate">
        <DefaultSelect
          value={v.fps === undefined ? undefined : String(v.fps)}
          options={LIMITS.fps.map((f) => ({ value: String(f), label: `${f} fps` }))}
          defaultLabel={`${g.video.fps} fps`}
          onChange={(f) => setVideo({ fps: f === undefined ? undefined : Number(f) })}
        />
      </SettingRow>
      <SettingRow label="Codec">
        <DefaultSelect<Codec>
          value={v.codec}
          options={CODECS.map((c) => ({
            value: c.value,
            label: playable.has(c.value) ? c.label : `${c.label} — can't play on this PC`,
            disabled: !playable.has(c.value) && c.value !== v.codec,
          }))}
          defaultLabel={labelOf(CODECS, g.video.codec)}
          onChange={(codec) => setVideo({ codec })}
        />
      </SettingRow>
      <SettingRow label="Quality">
        <DefaultSelect<Quality>
          value={v.quality}
          options={QUALITIES}
          defaultLabel={labelOf(QUALITIES, g.video.quality)}
          onChange={(quality) => setVideo({ quality })}
        />
      </SettingRow>
      <SettingRow label="Bitrate cap">
        <DefaultSelect
          value={v.bitrate_cap === undefined ? undefined : v.bitrate_cap === "auto" ? "auto" : "fixed"}
          options={[
            { value: "auto", label: "Automatic" },
            { value: "fixed", label: "Fixed cap" },
          ]}
          defaultLabel={g.video.bitrate_cap_mbps === null ? "Automatic" : `${g.video.bitrate_cap_mbps} Mbps`}
          onChange={(c) =>
            setVideo({ bitrate_cap: c === "fixed" ? (g.video.bitrate_cap_mbps ?? 40) : (c as "auto" | undefined) })
          }
        />
      </SettingRow>
      {typeof v.bitrate_cap === "number" && (
        <SettingRow label="Peak bitrate" stacked>
          <SliderWithValue
            value={v.bitrate_cap}
            min={capMin}
            max={capMax}
            step={1}
            format={(m) => `${m} Mbps`}
            onChange={(m) => setVideo({ bitrate_cap: m })}
          />
        </SettingRow>
      )}

      <GroupLabel>Audio</GroupLabel>
      <SettingRow label="Record computer audio">
        <DefaultSelect
          value={toOnOff(a.game_audio)}
          options={ON_OFF}
          defaultLabel={onOff(g.audio.game_audio)}
          onChange={(s) => setAudio({ game_audio: fromOnOff(s) })}
        />
      </SettingRow>
      <SettingRow label="What to record" disabled={!eff.audio.game_audio}>
        <DefaultSelect<AudioSourceKind>
          value={a.source}
          options={SOURCES}
          defaultLabel={labelOf(SOURCES, g.audio.source)}
          disabled={!eff.audio.game_audio}
          onChange={(source) => setAudio({ source })}
        />
      </SettingRow>
      <SettingRow label="Record microphone">
        <DefaultSelect
          value={toOnOff(a.mic)}
          options={ON_OFF}
          defaultLabel={onOff(g.audio.mic)}
          onChange={(s) => setAudio({ mic: fromOnOff(s) })}
        />
      </SettingRow>
      <SettingRow label="Add a mixed track first" disabled={!both}>
        <DefaultSelect
          value={toOnOff(a.mixed_track)}
          options={ON_OFF}
          defaultLabel={onOff(g.audio.mixed_track)}
          disabled={!both}
          onChange={(s) => setAudio({ mixed_track: fromOnOff(s) })}
        />
      </SettingRow>
      {both && eff.audio.mixed_track && (
        <>
          <DefaultSliderRow
            label={`${eff.audio.source === "game" ? "Game" : "Desktop"} volume in the mix`}
            value={a.game_volume}
            global={g.audio.game_volume}
            min={vMin}
            max={vMax}
            step={5}
            format={(p) => `${p}%`}
            onChange={(game_volume) => setAudio({ game_volume })}
          />
          <DefaultSliderRow
            label="Mic volume in the mix"
            value={a.mic_volume}
            global={g.audio.mic_volume}
            min={vMin}
            max={vMax}
            step={5}
            format={(p) => `${p}%`}
            onChange={(mic_volume) => setAudio({ mic_volume })}
          />
        </>
      )}
      <Hint>The microphone device and audio bitrate always come from the global Audio settings.</Hint>
    </>
  );
}

/** A select whose first choice, "Default (…)", means "not overridden". */
function DefaultSelect<T extends string>(props: {
  value: T | undefined;
  options: { value: T; label: string; disabled?: boolean }[];
  defaultLabel: string;
  onChange: (value: T | undefined) => void;
  disabled?: boolean;
}) {
  return (
    <Select
      value={props.value ?? DEFAULT}
      disabled={props.disabled}
      onValueChange={(s) => props.onChange(s === DEFAULT ? undefined : (s as T))}
    >
      <SelectTrigger className="w-64">
        <SelectValue />
      </SelectTrigger>
      <SelectContent>
        <SelectItem value={DEFAULT}>Default ({props.defaultLabel})</SelectItem>
        {props.options.map((o) => (
          <SelectItem key={o.value} value={o.value} disabled={o.disabled}>
            {o.label}
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  );
}

/** A slider setting with a "Custom" switch; off follows the global value. */
function DefaultSliderRow(props: {
  label: string;
  value: number | undefined;
  global: number;
  min: number;
  max: number;
  step: number;
  format: (n: number) => string;
  defaultFormat?: (n: number) => string;
  onChange: (value: number | undefined) => void;
}) {
  const custom = props.value !== undefined;
  const switchLabel = (
    <label className="flex items-center gap-2 text-sm text-muted-foreground">
      Custom
      <Switch checked={custom} onCheckedChange={(on) => props.onChange(on ? props.global : undefined)} />
    </label>
  );
  if (props.value === undefined) {
    return (
      <SettingRow label={props.label} description={`Default (${(props.defaultFormat ?? props.format)(props.global)})`}>
        {switchLabel}
      </SettingRow>
    );
  }
  return (
    <SettingRow label={props.label} stacked>
      <div className="flex items-center gap-4">
        <SliderWithValue {...props} value={props.value} />
        {switchLabel}
      </div>
    </SettingRow>
  );
}

function SliderWithValue(props: {
  value: number;
  min: number;
  max: number;
  step: number;
  format: (n: number) => string;
  onChange: (value: number) => void;
}) {
  return (
    <div className="flex flex-1 items-center gap-4">
      <Slider
        className="flex-1"
        min={props.min}
        max={props.max}
        step={props.step}
        value={[props.value]}
        onValueChange={([n]) => props.onChange(n ?? props.value)}
      />
      <span className="w-20 text-right font-mono text-sm">{props.format(props.value)}</span>
    </div>
  );
}
