import assert from "node:assert/strict";
import { test } from "node:test";
import type { Hunk } from "../../../generated/Hunk";
import { markedLines, oldLines } from "./markedLines.ts";

const hunk = (baseStart: number, baseLen: number, targetStart: number, targetLen: number): Hunk => ({
  baseStart,
  baseLen,
  targetStart,
  targetLen,
});

void test("added and changed lines carry their hunk's bar", () => {
  const marks = markedLines([hunk(2, 1, 2, 3), hunk(9, 0, 10, 1)], 20);
  assert.deepEqual(
    [...marks].map(([line, m]) => [line, m.hunk, m.kind]),
    [
      [2, 0, "bar"],
      [3, 0, "bar"],
      [4, 0, "bar"],
      [10, 1, "bar"],
    ],
  );
});

void test("removed lines mark the line that now follows them", () => {
  const marks = markedLines([hunk(5, 2, 5, 0)], 20);
  assert.deepEqual([...marks], [[5, { hunk: 0, kind: "removedAbove" }]]);
});

void test("lines removed at the end of the file mark the last line from below", () => {
  const marks = markedLines([hunk(18, 2, 18, 0)], 18);
  assert.deepEqual([...marks], [[17, { hunk: 0, kind: "removedBelow" }]]);
});

void test("a removal beside a changed line leaves the bar on that line", () => {
  const marks = markedLines([hunk(1, 1, 1, 0), hunk(3, 1, 1, 2)], 10);
  assert.equal(marks.get(1)?.kind, "bar");
  assert.equal(marks.get(1)?.hunk, 1);
});

void test("a text without lines has no marks", () => {
  assert.equal(markedLines([hunk(0, 3, 0, 0)], 0).size, 0);
});

void test("old lines are the hunk's base lines without line endings", () => {
  assert.deepEqual(oldLines("a\r\nb\r\nc\nd", { baseStart: 1, baseLen: 2 }), ["b", "c"]);
  assert.deepEqual(oldLines("a\nb", { baseStart: 1, baseLen: 0 }), []);
});
