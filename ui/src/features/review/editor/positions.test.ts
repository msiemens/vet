import assert from "node:assert/strict";
import { test } from "node:test";
import { type LineDoc, lineEnd, lineOf, lineStart, offsetAt, selectedLines, targetLineOfBase } from "./positions.ts";

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

void test("a selection covers the lines it touches, a caret its own line", () => {
  assert.deepEqual(selectedLines(doc, 1, 5), { start: 0, len: 3 });
  assert.deepEqual(selectedLines(doc, 5, 5), { start: 2, len: 1 });
  assert.deepEqual(selectedLines(doc, 4, 4), { start: 2, len: 1 });
});

void test("a selection ending at the start of a line does not include it", () => {
  assert.deepEqual(selectedLines(doc, 0, 4), { start: 0, len: 2 });
  // Up to the start of the empty line: only the first line.
  assert.deepEqual(selectedLines(doc, 0, 3), { start: 0, len: 1 });
  // A selection that starts at a line start keeps its line.
  assert.deepEqual(selectedLines(doc, 3, 4), { start: 1, len: 1 });
});

void test("a position maps to its offset", () => {
  assert.equal(offsetAt(doc, { line: 0, col: 1 }), 1);
  assert.equal(offsetAt(doc, { line: 2, col: 2 }), 6);
});

void test("a base line stands for the target line the hunks above it move it to, a deleted one for its hunk", () => {
  // Base lines 2..3 became target line 2 (one line fewer); one line was added before base line 5.
  const hunks = [
    { baseStart: 2, baseLen: 2, targetStart: 2, targetLen: 1 },
    { baseStart: 5, baseLen: 0, targetStart: 4, targetLen: 1 },
  ];
  assert.equal(targetLineOfBase(hunks, 1), 1);
  assert.equal(targetLineOfBase(hunks, 2), 2);
  assert.equal(targetLineOfBase(hunks, 3), 2);
  assert.equal(targetLineOfBase(hunks, 4), 3);
  assert.equal(targetLineOfBase(hunks, 5), 5);
  assert.equal(targetLineOfBase([], 7), 7);
});

void test("a position outside the document is clamped", () => {
  assert.equal(offsetAt(doc, { line: 0, col: 9 }), 2);
  assert.equal(offsetAt(doc, { line: 1, col: 4 }), 3);
  assert.equal(offsetAt(doc, { line: 9, col: 0 }), 7);
});
