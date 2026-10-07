import assert from "node:assert/strict";
import { test } from "node:test";
import { Stops, hunkAt, nearestHunk } from "./stops.ts";

// File 0: 1 card, 2 hunks (stops 0-2). File 1: nothing. File 2: 2 hunks (3-4). File 3: 1 card (5).
const stops = new Stops([
  { cards: 1, hunks: 2 },
  { cards: 0, hunks: 0 },
  { cards: 0, hunks: 2 },
  { cards: 1, hunks: 0 },
]);

void test("stops number the cards, then the hunks, of each file in turn", () => {
  assert.equal(stops.total, 6);
  assert.deepEqual(
    [0, 1, 2, 3].map((fi) => stops.start(fi)),
    [0, 3, 3, 5],
  );
});

void test("a stop is located by its file and its index there, skipping files without stops", () => {
  assert.deepEqual(
    [0, 1, 2, 3, 4, 5].map((s) => stops.locate(s)),
    [
      [0, 0],
      [0, 1],
      [0, 2],
      [2, 0],
      [2, 1],
      [3, 0],
    ],
  );
});

void test("a stop outside the range is clamped to the first or last one", () => {
  assert.equal(stops.clamp(-3), 0);
  assert.equal(stops.clamp(1), 1);
  assert.equal(stops.clamp(9), 5);
});

void test("the nearest stop of a file is its card, else the closest hunk, else its first stop", () => {
  assert.equal(stops.nearest(0, 1, 0, [10, 40], 0), 0);
  assert.equal(stops.nearest(0, 1, -1, [10, 40], 35), 2);
  assert.equal(stops.nearest(2, 0, -1, [10, 40], 12), 3);
  assert.equal(stops.nearest(3, 1, -1, [], 7), 5);
  assert.equal(stops.nearest(1, 0, -1, [], 7), undefined);
});

void test("the hunk at a line is the last one starting at or above it, else the first", () => {
  assert.equal(hunkAt([10, 20, 30], 25), 1);
  assert.equal(hunkAt([10, 20, 30], 30), 2);
  assert.equal(hunkAt([10, 20, 30], 3), 0);
});

void test("the nearest hunk is the closest start; on a tie the earlier one", () => {
  assert.equal(nearestHunk([10, 20], 14), 0);
  assert.equal(nearestHunk([10, 20], 15), 0);
  assert.equal(nearestHunk([10, 20], 16), 1);
  assert.equal(nearestHunk([], 5), -1);
});
