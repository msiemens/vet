// Run with `pnpm test` (node's built-in runner).
import assert from "node:assert/strict";
import { test } from "node:test";
import { bindingOf, canonicalSpec, isTextInput, matches, specOf, type KeyLike } from "./keymap.ts";

const key = (init: Partial<KeyLike>): KeyLike => ({
  key: "",
  code: "",
  ctrlKey: false,
  altKey: false,
  shiftKey: false,
  metaKey: false,
  ...init,
});

// German layout on Windows: AltGr is Ctrl+Alt and reports the AltGraph modifier.
const altGr = (k: string, code: string) =>
  key({ key: k, code, ctrlKey: true, altKey: true, getModifierState: (m) => m === "AltGraph" });

void test("AltGr characters never match Ctrl-Alt bindings", () => {
  for (const [k, code, spec] of [
    ["@", "KeyQ", "Ctrl-Alt-q"],
    ["€", "KeyE", "Ctrl-Alt-e"],
    ["{", "Digit7", "Ctrl-Alt-7"],
    ["[", "Digit8", "Ctrl-Alt-8"],
    ["]", "Digit9", "Ctrl-Alt-9"],
    ["}", "Digit0", "Ctrl-Alt-0"],
    ["\\", "Minus", "Ctrl-Alt-ß"],
    ["~", "BracketRight", "Ctrl-Alt-+"],
    ["|", "IntlBackslash", "Ctrl-Alt-<"],
  ]) {
    assert.ok(isTextInput(altGr(k, code)), k);
    assert.ok(!matches(spec, altGr(k, code)), k);
  }
});

void test("AltGr is detected even without the AltGraph modifier (Ctrl+Alt only)", () => {
  assert.ok(isTextInput(key({ key: "@", code: "KeyQ", ctrlKey: true, altKey: true })));
});

void test("macOS Option characters are text", () => {
  assert.ok(isTextInput(key({ key: "{", code: "Digit8", altKey: true })));
  assert.ok(isTextInput(key({ key: "ß", code: "KeyS", altKey: true })));
});

void test("dead keys and IME composition are text", () => {
  assert.ok(isTextInput(key({ key: "Dead", code: "Quote" })));
  assert.ok(isTextInput(key({ key: "Process", code: "KeyA", keyCode: 229 })));
  assert.ok(isTextInput(key({ key: "a", code: "KeyA", isComposing: true })));
});

void test("real Ctrl+Alt bindings still fire", () => {
  assert.ok(matches("Ctrl-Alt-ArrowDown", key({ key: "ArrowDown", code: "ArrowDown", ctrlKey: true, altKey: true })));
  assert.ok(matches("Ctrl-Alt-s", key({ key: "s", code: "KeyS", ctrlKey: true, altKey: true })));
  assert.ok(!matches("Ctrl-Alt-s", key({ key: "s", code: "KeyS", ctrlKey: true })));
});

void test("Mod becomes the platform's modifier, and labels read as the menus show them", () => {
  assert.deepEqual(bindingOf("Mod-b", "Meta"), { spec: "Meta-b", label: "⌘B", chord: "Cmd+B" });
  assert.deepEqual(bindingOf("Mod-b", "Ctrl"), { spec: "Ctrl-b", label: "Ctrl+B", chord: "Ctrl+B" });
  assert.equal(bindingOf("Ctrl-Alt-ArrowDown", "Ctrl").label, "Ctrl+Alt+Down");
  assert.equal(bindingOf("Mod-Enter", "Ctrl").label, "Ctrl+Enter");
  assert.equal(bindingOf("Mod-Enter", "Meta").label, "⌘↩");
});

void test("macOS writes a key as glyphs in its own order of modifiers, while a native menu still reads the plain chord", () => {
  const mac = (spec: string) => bindingOf(spec, "Meta");
  assert.equal(mac("Ctrl-Alt-ArrowDown").label, "⌃⌥↓");
  assert.equal(mac("Ctrl-Alt-ArrowDown").chord, "Ctrl+Alt+Down");
  assert.equal(mac("Meta-Shift-o").label, "⇧⌘O");
  assert.equal(mac("Mod-Shift-o").chord, "Cmd+Shift+O");
  assert.equal(mac("Alt-1").label, "⌥1");
  assert.equal(mac("Meta-Shift-BracketLeft").label, "⇧⌘[");
  assert.equal(mac("Meta-Shift-BracketLeft").chord, "Cmd+Shift+[");
  for (const plain of ["Space", "F4", "[", "r"])
    assert.equal(mac(plain).label, plain.length === 1 ? plain.toUpperCase() : plain);
  assert.equal(bindingOf("Alt-ArrowLeft", "Ctrl").label, "Alt+Left");
});

void test("a bracket with Shift is told by its place on the keyboard, not by the character the layout types", () => {
  const shifted = (init: Partial<KeyLike>) => key({ metaKey: true, shiftKey: true, ...init });
  assert.ok(matches("Meta-Shift-BracketLeft", shifted({ key: "{", code: "BracketLeft" })));
  assert.ok(matches("Meta-Shift-BracketRight", shifted({ key: "}", code: "BracketRight" })));
  assert.ok(!matches("Meta-Shift-BracketLeft", shifted({ key: "}", code: "BracketRight" })));
  assert.ok(!matches("Meta-Shift-BracketLeft", key({ metaKey: true, key: "[", code: "BracketLeft" })));
});

void test("Option types nothing outside a text field, so a plain Alt chord can be a shortcut there; AltGr and composition never can", () => {
  const option1 = key({ key: "¡", code: "Digit1", altKey: true });
  assert.ok(isTextInput(option1));
  assert.ok(!isTextInput(option1, true));
  assert.ok(matches("Alt-1", option1, true));
  assert.ok(!matches("Alt-1", option1));
  assert.ok(isTextInput(altGr("@", "KeyQ"), true));
  assert.ok(isTextInput(key({ key: "Dead", code: "Quote", altKey: true }), true));
  assert.ok(isTextInput(key({ key: "a", code: "KeyA", altKey: true, isComposing: true }), true));
});

void test("Space toggles the current item, and only on its own", () => {
  assert.ok(matches("Space", key({ key: " ", code: "Space" })));
  assert.ok(!matches("Space", key({ key: " ", code: "Space", shiftKey: true })));
  assert.ok(!matches("Space", key({ key: " ", code: "Space", isComposing: true })));
  assert.ok(!matches("Space", key({ key: "s", code: "KeyS" })));
});

void test("punctuation matches the character the layout types, on its own", () => {
  assert.ok(matches("[", key({ key: "[", code: "BracketLeft" })));
  assert.ok(!matches("[", key({ key: "]", code: "BracketRight" })));
  assert.ok(!matches("[", key({ key: "[", code: "BracketLeft", ctrlKey: true })));
  assert.equal(bindingOf("]", "Ctrl").label, "]");
});

void test("a recorded key reads back as the spec that matches it, however the layout types it", () => {
  const events = [
    key({ key: "ß", code: "KeyS", ctrlKey: true, shiftKey: true }),
    key({ key: "ArrowDown", code: "ArrowDown", ctrlKey: true, altKey: true }),
    key({ key: " ", code: "Space" }),
    key({ key: "F4", code: "F4", metaKey: true }),
    key({ key: "¡", code: "Digit1", ctrlKey: true }),
  ];
  assert.deepEqual(events.map(specOf), ["Ctrl-Shift-s", "Ctrl-Alt-ArrowDown", "Space", "Meta-F4", "Ctrl-1"]);
  for (const e of events) assert.ok(matches(specOf(e) ?? "", e));
});

void test("text input and keys that are not a shortcut cannot be recorded", () => {
  assert.equal(specOf(altGr("@", "KeyQ")), null);
  assert.equal(specOf(key({ key: "{", code: "Digit8", altKey: true })), null);
  assert.equal(specOf(key({ key: "Dead", code: "Quote" })), null);
  assert.equal(specOf(key({ key: "a", code: "KeyA", isComposing: true })), null);
  assert.equal(specOf(key({ key: "Control", code: "ControlLeft", ctrlKey: true })), null);
  assert.equal(specOf(key({ key: ".", code: "Period", ctrlKey: true })), null);
});

void test("equal bindings compare equal whatever the order of their modifiers or the case of the letter", () => {
  assert.equal(canonicalSpec("Alt-Ctrl-S"), canonicalSpec("Ctrl-Alt-s"));
  assert.notEqual(canonicalSpec("Ctrl-Alt-s"), canonicalSpec("Ctrl-s"));
});
