import assert from "node:assert/strict";
import { test } from "node:test";
import type { KeyLike } from "../../lib/keymap.ts";
import { tabStep } from "./tabKey.ts";

const press = (key: string, mods: Partial<KeyLike> = {}): KeyLike => ({
  key,
  code: key,
  ctrlKey: false,
  altKey: false,
  shiftKey: false,
  metaKey: false,
  ...mods,
});

void test("Tab and Ctrl+Tab move to the next tab, with Shift to the previous", () => {
  assert.equal(tabStep(press("Tab")), 1);
  assert.equal(tabStep(press("Tab", { shiftKey: true })), -1);
  assert.equal(tabStep(press("Tab", { ctrlKey: true })), 1);
  assert.equal(tabStep(press("Tab", { ctrlKey: true, shiftKey: true })), -1);
});

void test("other chords on Tab, and other keys, leave the tab alone", () => {
  assert.equal(tabStep(press("Tab", { metaKey: true })), 0);
  assert.equal(tabStep(press("Tab", { altKey: true })), 0);
  assert.equal(tabStep(press("Enter", { ctrlKey: true })), 0);
});
