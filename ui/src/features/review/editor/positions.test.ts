import assert from "node:assert/strict";
import { test } from "node:test";
import { type LineDoc, lineEnd, lineOf, lineStart, offsetAt, selectedLines } from "./positions.ts";

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
