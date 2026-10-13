import assert from "node:assert/strict";
import { test } from "node:test";
import type { VersionInfo } from "../generated/VersionInfo";
import { chooseRow, defaultRange, normalised, pushesOf } from "./timelineRange.ts";

/** Versions 1..`count`, with the one numbered `reviewed` the last one something was marked in. */
const versions = (count: number, reviewed?: number): VersionInfo[] =>
  Array.from({ length: count }, (_, i) => ({
    n: i + 1,
    at: 0,
    origin: "settled",
    tip: null,
    base: null,
    reviewedHere: i + 1 === reviewed,
    files: [],
  }));

void test("opening the timeline compares the last reviewed version with the latest", () => {
  assert.deepEqual(defaultRange(versions(17, 14)), { from: 14, to: 17 });
});

void test("without a mark the range starts at the first version", () => {
  assert.deepEqual(defaultRange(versions(4)), { from: 1, to: 4 });
});

void test("nothing is compared when the last reviewed version is the latest, or there is only one", () => {
  assert.equal(defaultRange(versions(5, 5)), undefined);
  assert.equal(defaultRange(versions(1)), undefined);
  assert.equal(defaultRange([]), undefined);
});

void test("a click shows that push, compared with the version before it", () => {
  assert.deepEqual(chooseRow(versions(17), undefined, 15), { from: 14, to: 15 });
  assert.deepEqual(chooseRow(versions(17), undefined, 17), { from: 16, to: 17 });
});

void test("the first push is compared with the base, which is version 0", () => {
  assert.deepEqual(chooseRow(versions(3), undefined, 1), { from: 0, to: 1 });
});

void test("clicking the only push shown goes back to the whole changeset, another push replaces it", () => {
  const second = { from: 1, to: 2 };
  assert.equal(chooseRow(versions(3), second, 2), undefined);
  assert.deepEqual(chooseRow(versions(3), second, 3), { from: 2, to: 3 });
  assert.deepEqual(chooseRow(versions(3), { from: 1, to: 3 }, 3), { from: 2, to: 3 });
});

void test("a shift-click shows the block of pushes between the two, whichever was clicked first", () => {
  assert.deepEqual(chooseRow(versions(17), undefined, 17, 14), { from: 13, to: 17 });
  assert.deepEqual(chooseRow(versions(17), undefined, 12, 15), { from: 11, to: 15 });
  assert.deepEqual(chooseRow(versions(17), { from: 14, to: 15 }, 15, 15), { from: 14, to: 15 });
});

void test("every push is the whole changeset, which is no range", () => {
  assert.equal(chooseRow(versions(3), undefined, 3, 1), undefined);
  assert.equal(chooseRow(versions(1), undefined, 1), undefined);
  assert.deepEqual(chooseRow(versions(3), undefined, 2, 1), { from: 0, to: 2 });
});

void test("a range that covers every push is the changeset, a narrower one stays as chosen", () => {
  assert.equal(normalised({ from: 0, to: 4 }, 4), undefined);
  assert.deepEqual(normalised({ from: 2, to: 3 }, 3), { from: 2, to: 3 });
  assert.deepEqual(normalised({ from: 0, to: 3 }, 4), { from: 0, to: 3 });
  assert.equal(normalised(undefined, 4), undefined);
});

void test("a range shows the pushes after the version it starts at", () => {
  assert.deepEqual(pushesOf({ from: 14, to: 17 }), { first: 15, last: 17 });
  assert.deepEqual(pushesOf({ from: 0, to: 2 }), { first: 1, last: 2 });
});
