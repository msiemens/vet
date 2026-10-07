// Run with `pnpm test` (node's built-in runner).
import assert from "node:assert/strict";
import { test } from "node:test";
import { isTextInput, KEYS, matches, type KeyLike } from "./keymap.ts";

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

void test("Ctrl-Alt-S is Settings", () => {
  assert.ok(matches(KEYS.settings.spec, key({ key: "s", code: "KeyS", ctrlKey: true, altKey: true })));
  assert.ok(!matches(KEYS.settings.spec, altGr("ß", "KeyS")));
});

void test("a binding's label shows the keys its spec matches", () => {
  assert.equal(KEYS.settings.label, "Ctrl+Alt+S");
  assert.equal(KEYS.nextHunk.label, "Ctrl+Alt+Down");
  assert.equal(KEYS.toggleCurrent.label, "Space");
  assert.equal(KEYS.jumpToSource.label, "F4");
});

void test("Space toggles the current item, and only on its own", () => {
  assert.ok(matches("Space", key({ key: " ", code: "Space" })));
  assert.ok(!matches("Space", key({ key: " ", code: "Space", shiftKey: true })));
  assert.ok(!matches("Space", key({ key: " ", code: "Space", isComposing: true })));
  assert.ok(!matches("Space", key({ key: "s", code: "KeyS" })));
});

void test("F4 is Jump to Source, and only without modifiers", () => {
  assert.ok(matches("F4", key({ key: "F4", code: "F4" })));
  assert.ok(!matches("F4", key({ key: "F4", code: "F4", altKey: true })));
});
