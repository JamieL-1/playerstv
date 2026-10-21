import { describe, expect, it } from "vitest";
import { hotkeyFromKeyPress, hotkeyParts, type KeyPress } from "./hotkeys";

const press = (code: string, mods: Partial<KeyPress> = {}): KeyPress => ({
  code,
  ctrlKey: false,
  altKey: false,
  shiftKey: false,
  metaKey: false,
  ...mods,
});

describe("hotkeyFromKeyPress", () => {
  it("builds tray-compatible strings", () => {
    expect(hotkeyFromKeyPress(press("F10", { altKey: true }))).toEqual({ kind: "ok", hotkey: "Alt+F10" });
    expect(hotkeyFromKeyPress(press("KeyS", { ctrlKey: true, shiftKey: true }))).toEqual({
      kind: "ok",
      hotkey: "Ctrl+Shift+S",
    });
    expect(hotkeyFromKeyPress(press("Digit5", { altKey: true }))).toEqual({ kind: "ok", hotkey: "Alt+5" });
    expect(hotkeyFromKeyPress(press("Numpad1", { ctrlKey: true }))).toEqual({ kind: "ok", hotkey: "Ctrl+Numpad1" });
  });

  it("keeps a fixed modifier order", () => {
    const all = press("KeyK", { metaKey: true, shiftKey: true, altKey: true, ctrlKey: true });
    expect(hotkeyFromKeyPress(all)).toEqual({ kind: "ok", hotkey: "Ctrl+Alt+Shift+Super+K" });
  });

  it("waits while only modifiers are held", () => {
    expect(hotkeyFromKeyPress(press("AltLeft", { altKey: true }))).toEqual({ kind: "pending" });
    expect(hotkeyFromKeyPress(press("ControlRight", { ctrlKey: true }))).toEqual({ kind: "pending" });
  });

  it("allows F-keys alone but not letters", () => {
    expect(hotkeyFromKeyPress(press("F9"))).toEqual({ kind: "ok", hotkey: "F9" });
    expect(hotkeyFromKeyPress(press("F24"))).toEqual({ kind: "ok", hotkey: "F24" });
    expect(hotkeyFromKeyPress(press("KeyA"))).toEqual({ kind: "needs-modifier", key: "A" });
    expect(hotkeyFromKeyPress(press("Space"))).toEqual({ kind: "needs-modifier", key: "Space" });
  });
});

describe("hotkeyParts", () => {
  it("splits for key caps", () => {
    expect(hotkeyParts("Ctrl+Shift+S")).toEqual(["Ctrl", "Shift", "S"]);
    expect(hotkeyParts("")).toEqual([]);
  });
});
