import assert from "node:assert/strict";
import { test } from "node:test";
import { type BarFacts, barChange } from "./barChange.ts";

const actions = {};
const bar = (over: Partial<BarFacts> = {}): BarFacts => ({
  index: 2,
  text: "@@ −1,2 +1,3 @@",
  state: "unseen",
  oldBaseLines: 0,
  baseDiffOpen: false,
  current: false,
  actions,
  ...over,
});

void test("a bar that only becomes or ceases to be current keeps its elements", () => {
  assert.equal(barChange(bar(), bar({ current: true })), "outline");
  assert.equal(barChange(bar({ current: true }), bar()), "outline");
});

void test("an unchanged bar needs nothing", () => {
  assert.equal(barChange(bar(), bar()), "none");
});

void test("a bar whose hunk state or handlers changed is drawn anew, current or not", () => {
  assert.equal(barChange(bar(), bar({ state: "seen" })), "redraw");
  assert.equal(barChange(bar(), bar({ state: "seen", current: true })), "redraw");
  assert.equal(barChange(bar(), bar({ baseDiffOpen: true })), "redraw");
  assert.equal(barChange(bar(), bar({ actions: {} })), "redraw");
  assert.equal(barChange(bar(), bar({ actions: null })), "redraw");
});
