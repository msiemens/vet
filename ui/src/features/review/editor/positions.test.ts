import assert from "node:assert/strict";
import { test } from "node:test";
import { type LineDoc, lineEnd, lineOf, lineStart, offsetAt, selectedLines, targetLine } from "./positions.ts";

/** "ab\n\ncde": lines at 0..2, 3..3, 4..7. */
const spans = [
  { from: 0, to: 2 },
  { from: 3, to: 3 },
  { from: 4, to: 7 },
];
const doc: LineDoc = {
  lines: 3,
  line: (n) => spans[n - 1],
  lineAt: (pos) => ({ number: spans.findIndex((s) => pos <= s.to) + 1 }),
};

void test("lines map to offsets and back; a line past the end is the last", () => {
  assert.equal(lineStart(doc, 2), 4);
  assert.equal(lineEnd(doc, 1), 3);
  assert.equal(lineStart(doc, 9), 4);
  assert.equal(lineEnd(doc, 9), 7);
  assert.equal(lineOf(doc, 0), 0);
  assert.equal(lineOf(doc, 3), 1);
  assert.equal(lineOf(doc, 5), 2);
});

void test("a selection ending at the start of a line does not include it", () => {
  assert.deepEqual(selectedLines(3, 5, false), { start: 3, len: 3 });
  assert.deepEqual(selectedLines(3, 5, true), { start: 3, len: 2 });
  assert.deepEqual(selectedLines(3, 3, true), { start: 3, len: 1 });
});

void test("a position maps to its offset", () => {
  assert.equal(offsetAt(doc, { line: 0, col: 1 }), 1);
  assert.equal(offsetAt(doc, { line: 2, col: 2 }), 6);
});

void test("a position outside the document is clamped", () => {
  assert.equal(offsetAt(doc, { line: 0, col: 9 }), 2);
  assert.equal(offsetAt(doc, { line: 1, col: 4 }), 3);
  assert.equal(offsetAt(doc, { line: 9, col: 0 }), 7);
});

const hunk = (baseStart: number, baseLen: number, targetStart: number, targetLen: number) => ({
  baseStart,
  baseLen,
  targetStart,
  targetLen,
});

void test("the target side faces itself", () => {
  assert.equal(targetLine([hunk(10, 2, 10, 1)], "target", 7), 7);
});

void test("a base line faces the hunk it was changed in, or where it shifted to", () => {
  // Lines 10..12 of the base became one target line at 10; two lines were added at 17.
  const hunks = [hunk(10, 2, 10, 1), hunk(18, 0, 17, 2)];
  assert.equal(targetLine(hunks, "base", 5), 5);
  assert.equal(targetLine(hunks, "base", 10), 10); // inside the first hunk
  assert.equal(targetLine(hunks, "base", 11), 10);
  assert.equal(targetLine(hunks, "base", 14), 13); // one line fewer after it
  assert.equal(targetLine(hunks, "base", 30), 31); // and two more after both
  assert.equal(targetLine([], "base", 5), 5);
});

void test("a replaced base line faces the end of its replacement", () => {
  assert.equal(targetLine([hunk(4, 2, 4, 5)], "base", 4), 8);
});

void test("a pure deletion's base lines face the line that took their place", () => {
  const hunks = [hunk(5, 3, 5, 0)];
  assert.equal(targetLine(hunks, "base", 6), 5);
  assert.equal(targetLine(hunks, "base", 0), 0);
  assert.equal(targetLine([hunk(0, 2, 0, 0)], "base", 1), 0);
});
