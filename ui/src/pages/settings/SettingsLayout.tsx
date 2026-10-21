import { X } from "lucide-react";
import { useEffect } from "react";
import { NavLink, Navigate, useNavigate, useParams } from "react-router";
import { Button } from "@/components/ui/button";
import { ScrollArea } from "@/components/ui/scroll-area";
import { cn } from "@/lib/utils";
import { AppearanceSection } from "./AppearanceSection";
import { AudioSection } from "./AudioSection";
import { GamesSection } from "./GamesSection";
import { HotkeysSection, RecordingSection } from "./RecordingSection";
import { SettingsProvider, useSettings } from "./SettingsContext";
import { StorageSection } from "./StorageSection";
import { VideoSection } from "./VideoSection";

interface SectionDef {
  id: string;
  label: string;
  Component: () => React.JSX.Element;
}

const GROUPS: { label: string; items: SectionDef[] }[] = [
  {
    label: "Recording settings",
    items: [
      { id: "recording", label: "Recording", Component: RecordingSection },
      { id: "video", label: "Video", Component: VideoSection },
      { id: "audio", label: "Audio", Component: AudioSection },
      { id: "games", label: "Games", Component: GamesSection },
      { id: "hotkeys", label: "Hotkeys", Component: HotkeysSection },
      { id: "storage", label: "Storage", Component: StorageSection },
    ],
  },
  {
    label: "App settings",
    items: [{ id: "appearance", label: "Appearance", Component: AppearanceSection }],
  },
];

const SECTIONS = GROUPS.flatMap((g) => g.items);

/** Full-window settings, like Discord's User Settings. */
export function SettingsLayout() {
  return (
    <SettingsProvider>
      <SettingsScreen />
    </SettingsProvider>
  );
}

function SettingsScreen() {
  const { section } = useParams();
  const navigate = useNavigate();
  const { dirty } = useSettings();
  const current = SECTIONS.find((s) => s.id === section);

  // Esc closes, as in Discord (unless there are unsaved changes).
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !dirty && !e.defaultPrevented) navigate("/");
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [dirty, navigate]);

  if (!current) return <Navigate to="/settings/recording" replace />;
  const Section = current.Component;

  return (
    <div className="flex h-screen">
      <nav className="flex w-[280px] shrink-0 justify-end bg-sidebar py-14 pr-2">
        <div className="w-[200px] space-y-6">
          {GROUPS.map((g) => (
            <div key={g.label}>
              <div className="mb-1 px-2.5 text-xs font-bold uppercase tracking-wide text-muted-foreground">
                {g.label}
              </div>
              {g.items.map((item) => (
                <NavLink
                  key={item.id}
                  to={`/settings/${item.id}`}
                  className={cn(
                    "block rounded-md px-2.5 py-1.5 text-[15px] font-medium text-muted-foreground hover:bg-accent hover:text-foreground",
                    section === item.id && "bg-selected text-foreground",
                  )}
                >
                  {item.label}
                </NavLink>
              ))}
            </div>
          ))}
        </div>
      </nav>
      <div className="relative flex min-w-0 flex-1 bg-background">
        <ScrollArea className="h-screen flex-1">
          <div className="max-w-[740px] px-10 pb-28 pt-14">
            <Section />
          </div>
        </ScrollArea>
        <div className="flex flex-col items-center gap-1 pr-6 pt-14">
          <Button
            variant="outline"
            size="icon"
            className="size-9 rounded-full"
            aria-label="Close settings"
            disabled={dirty}
            onClick={() => navigate("/")}
          >
            <X />
          </Button>
          <span className="text-xs font-semibold text-muted-foreground">ESC</span>
        </div>
        <UnsavedBar />
      </div>
    </div>
  );
}

/** Discord's "Careful — you have unsaved changes!" bar. */
function UnsavedBar() {
  const { dirty, saving, save, reset } = useSettings();
  return (
    <div
      className={cn(
        "pointer-events-none absolute inset-x-0 bottom-0 flex justify-center pb-5 transition-all duration-200",
        dirty ? "translate-y-0 opacity-100" : "translate-y-6 opacity-0",
      )}
    >
      <div className="pointer-events-auto flex w-full max-w-[700px] items-center gap-3 rounded-lg bg-popover px-4 py-2.5 shadow-xl">
        <span className="flex-1 font-medium">Careful — you have unsaved changes!</span>
        <Button variant="link" className="text-foreground" onClick={reset} disabled={saving}>
          Reset
        </Button>
        <Button className="bg-green-600 text-white hover:bg-green-700" onClick={save} disabled={saving}>
          {saving ? "Saving…" : "Save Changes"}
        </Button>
      </div>
    </div>
  );
}
