import assert from "node:assert/strict";
import { test } from "node:test";
import { chooseGroupItem, groupMenu, menuChanged } from "./problemMenu.ts";

const OFF = { unseenOnly: false, changedOnly: false };

void test("one grouping is checked, and origin is off until the origins are known", () => {
  const items = groupMenu("file", OFF, false);
  const radios = items.filter((i) => i.radio);
  assert.deepEqual(
    radios.filter((i) => i.checked).map((i) => i.label),
    ["File"],
  );
  assert.equal(radios.find((i) => i.label === "Origin, then file")?.enabled, false);
  assert.ok(groupMenu("file", OFF, true).every((i) => i.enabled !== false));
});

void test("the filters show as checked as they are set", () => {
  const items = groupMenu("origin", { unseenOnly: true, changedOnly: false }, true);
  assert.equal(items.find((i) => i.label === "Problems in hunks not seen")?.checked, true);
  assert.equal(items.find((i) => i.label === "Problems on changed lines")?.checked, false);
});

void test("choosing a grouping sets it, choosing a filter flips only that filter", () => {
  const items = groupMenu("origin", OFF, true);
  const id = (label: string) => items.find((i) => i.label === label)?.id ?? "";
  assert.equal(chooseGroupItem(id("Severity"), "origin", OFF).grouping, "severity");
  const flipped = chooseGroupItem(id("Problems on changed lines"), "origin", { unseenOnly: true, changedOnly: false });
  assert.deepEqual(flipped, { grouping: "origin", unseenOnly: true, changedOnly: true });
});

void test("the menu button marks any setting that differs from the default", () => {
  assert.equal(menuChanged("origin", OFF), false);
  assert.equal(menuChanged("file", OFF), true);
  assert.equal(menuChanged("origin", { unseenOnly: false, changedOnly: true }), true);
});
