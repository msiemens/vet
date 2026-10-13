import assert from "node:assert/strict";
import { test } from "node:test";
import { diffSegments, seenSegments } from "./seenMeter.ts";

const values = (segments: { value: number }[]) => segments.map((s) => s.value);

void test("what is seen only against an old base is striped, never solid amber", () => {
  const [, old] = seenSegments(30, 10);
  assert.match(old.color, /stripes/);
});

void test("a diffstat bar is covered from the left by what is seen", () => {
  // 3 added of 4 changed lines: green to 75 %, red for the rest.
  assert.deepEqual(values(diffSegments(3, 1, 0, 0)), [0, 0, 75, 25]);
  assert.deepEqual(values(diffSegments(3, 1, 50, 0)), [50, 0, 25, 25]);
  assert.deepEqual(values(diffSegments(3, 1, 90, 0)), [90, 0, 0, 10]);
});

void test("a fully reviewed diffstat bar shows no green or red", () => {
  assert.deepEqual(values(diffSegments(3, 1, 100, 0)), [100, 0, 0, 0]);
  assert.deepEqual(values(diffSegments(3, 1, 70, 30)), [70, 30, 0, 0]);
});

void test("a diffstat bar of no changed lines stays neutral", () => {
  assert.deepEqual(values(diffSegments(0, 0, 0, 0)), [0, 0, 0, 0]);
});
