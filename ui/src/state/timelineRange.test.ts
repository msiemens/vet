import assert from "node:assert/strict";
import { test } from "node:test";
import type { VersionInfo } from "../generated/VersionInfo";
import { defaultRange, rowRange, withTo } from "./timelineRange.ts";

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

void test("opening the timeline compares the last reviewed version with the latest, live", () => {
  assert.deepEqual(defaultRange(versions(17, 14)), { from: 14, to: 17, live: true });
});

void test("without a mark the range starts at the first version", () => {
  assert.deepEqual(defaultRange(versions(4)), { from: 1, to: 4, live: true });
});

void test("nothing is compared when the last reviewed version is the latest, or there is only one", () => {
  assert.equal(defaultRange(versions(5, 5)), undefined);
  assert.equal(defaultRange(versions(1)), undefined);
  assert.equal(defaultRange([]), undefined);
});

void test("a click compares a version with the one before it, live only for the latest", () => {
  assert.deepEqual(rowRange(versions(17), 15), { from: 14, to: 15, live: false });
  assert.deepEqual(rowRange(versions(17), 17), { from: 16, to: 17, live: true });
});

void test("the first version has none before it to compare with", () => {
  assert.equal(rowRange(versions(3), 1), undefined);
});

void test("a second row runs the range between the two, whichever was clicked first", () => {
  assert.deepEqual(rowRange(versions(17), 17, 14), { from: 14, to: 17, live: true });
  assert.deepEqual(rowRange(versions(17), 12, 15), { from: 12, to: 15, live: false });
  assert.deepEqual(rowRange(versions(17), 15, 15), { from: 14, to: 15, live: false });
});

void test("moving the end to the latest version makes it live, to an older one pins it", () => {
  const live = { from: 14, to: 17, live: true };
  assert.deepEqual(withTo(live, 15, 17), { from: 14, to: 15, live: false });
  assert.deepEqual(withTo({ from: 14, to: 15, live: false }, 17, 17), live);
});
