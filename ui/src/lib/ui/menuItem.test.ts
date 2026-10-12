import assert from "node:assert/strict";
import { test } from "node:test";
import { type MenuItem, mnemonicTarget } from "./menuItem.ts";

const menu = (id: string, label: string, enabled?: boolean): MenuItem => ({
  id,
  label,
  children: [{ id: `${id}.x`, label: "X" }],
  ...(enabled === undefined ? {} : { enabled }),
});

const bar = [
  menu("file", "File"),
  menu("view", "View"),
  menu("changeset", "Changeset"),
  menu("review", "Review"),
  menu("navigate", "Navigate"),
  menu("help", "Help"),
];

void test("a letter picks the menu of the bar that starts with it, in either case, and the six menus have six letters", () => {
  assert.deepEqual(
    ["f", "v", "c", "r", "n", "h"].map((l) => mnemonicTarget(bar, l)?.id),
    ["file", "view", "changeset", "review", "navigate", "help"],
  );
  assert.equal(mnemonicTarget(bar, "F")?.id, "file");
});

void test("a letter that starts no menu, a key that is no letter and a disabled menu pick nothing", () => {
  assert.equal(mnemonicTarget(bar, "x"), undefined);
  assert.equal(mnemonicTarget(bar, "ArrowDown"), undefined);
  assert.equal(mnemonicTarget(bar, "Enter"), undefined);
  assert.equal(mnemonicTarget([menu("file", "File", false)], "f"), undefined);
  assert.equal(mnemonicTarget([{ id: "x", label: "Fetch" }], "f"), undefined, "an item that opens no menu");
});
