import assert from "node:assert/strict";
import { test } from "node:test";
import { windowRanges } from "./windowing.ts";

// Ten sections of 100px each.
const indexAt = (y: number) => Math.min(9, Math.max(0, Math.floor(y / 100)));

void test("once scrolling has stopped, a screen above and two below get editors", () => {
  // A 200px viewport at 500px: from 300px up to 900px.
  assert.deepEqual(windowRanges(indexAt, 500, 200, true).near, [3, 9]);
});

void test("while scrolling, only the sections in view get editors", () => {
  assert.deepEqual(windowRanges(indexAt, 500, 200, false).near, [5, 7]);
});

void test("editors outside two screens above and three below are dropped", () => {
  assert.deepEqual(windowRanges(indexAt, 500, 200, true).far, [1, 9]);
  assert.deepEqual(windowRanges(indexAt, 500, 100, false).far, [3, 8]);
});

void test("near the top the range starts at the first section", () => {
  const { near, far } = windowRanges(indexAt, 50, 200, true);
  assert.deepEqual([near[0], far[0]], [0, 0]);
});
