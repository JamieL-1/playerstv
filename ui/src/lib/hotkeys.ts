// Turning a key press into the hotkey string the tray registers. The tray
// parses these with the `global-hotkey` crate, which accepts W3C
// KeyboardEvent.code names (KeyA, Digit1, F10, Numpad1, Space, …) and the
// modifiers Ctrl/Alt/Shift/Super.

export interface KeyPress {
  code: string;
  ctrlKey: boolean;
  altKey: boolean;
  shiftKey: boolean;
  metaKey: boolean;
}

export type HotkeyResult =
  | { kind: "ok"; hotkey: string }
  /** Only modifiers held so far: keep waiting. */
  | { kind: "pending" }
  /** A plain letter/number as a global hotkey would block typing it everywhere. */
  | { kind: "needs-modifier"; key: string };

const MODIFIER_CODES = new Set([
  "ControlLeft",
  "ControlRight",
  "AltLeft",
  "AltRight",
  "ShiftLeft",
  "ShiftRight",
  "MetaLeft",
  "MetaRight",
  "OSLeft",
  "OSRight",
]);

/** Keys fine to use on their own (they don't type anything). */
function standalone(code: string): boolean {
  return /^F([1-9]|1\d|2[0-4])$/.test(code) || ["Pause", "ScrollLock", "Insert", "PrintScreen"].includes(code);
}

/** Display/parse name for a key: KeyA → A, Digit5 → 5, others unchanged. */
export function keyName(code: string): string {
  if (/^Key[A-Z]$/.test(code)) return code.slice(3);
  if (/^Digit\d$/.test(code)) return code.slice(5);
  return code;
}

export function hotkeyFromKeyPress(e: KeyPress): HotkeyResult {
  if (MODIFIER_CODES.has(e.code)) return { kind: "pending" };
  const mods = [e.ctrlKey && "Ctrl", e.altKey && "Alt", e.shiftKey && "Shift", e.metaKey && "Super"].filter(
    (m): m is string => Boolean(m),
  );
  const key = keyName(e.code);
  if (mods.length === 0 && !standalone(e.code)) return { kind: "needs-modifier", key };
  return { kind: "ok", hotkey: [...mods, key].join("+") };
}

/** "Alt+F10" → ["Alt", "F10"], for rendering as key caps. */
export function hotkeyParts(hotkey: string): string[] {
  return hotkey
    .split("+")
    .map((p) => p.trim())
    .filter(Boolean);
}
