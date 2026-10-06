import assert from "node:assert/strict";
import { test } from "node:test";
import type { SemanticLocation } from "../../../generated/SemanticLocation.ts";
import { type LineDoc, baseLineToTarget, offsetAt, pickLocation } from "./positions.ts";

/** "ab\n\ncde": lines at 0..2, 3..3, 4..7. */
const doc: LineDoc = {
  lines: 3,
  line: (n) =>
    [
      { from: 0, to: 2 },
      { from: 3, to: 3 },
      { from: 4, to: 7 },
    ][n - 1],
};

void test("a position maps to its offset", () => {
  assert.equal(offsetAt(doc, { line: 0, col: 1 }), 1);
  assert.equal(offsetAt(doc, { line: 2, col: 2 }), 6);
});

void test("a position outside the document is clamped", () => {
  assert.equal(offsetAt(doc, { line: 0, col: 9 }), 2);
  assert.equal(offsetAt(doc, { line: 1, col: 4 }), 3);
  assert.equal(offsetAt(doc, { line: 9, col: 0 }), 7);
});

// Base lines 2..3 (two) became target lines 2..4 (three); line 10 was inserted (one line); line 20 deleted.
const hunks = [
  { baseStart: 2, baseLen: 2, targetStart: 2, targetLen: 3 },
  { baseStart: 8, baseLen: 0, targetStart: 9, targetLen: 1 },
  { baseStart: 15, baseLen: 1, targetStart: 17, targetLen: 0 },
];

void test("base lines before the first hunk stay", () => {
  assert.equal(baseLineToTarget(hunks, 0), 0);
  assert.equal(baseLineToTarget(hunks, 1), 1);
});

void test("a replaced or deleted base line maps to its hunk", () => {
  assert.equal(baseLineToTarget(hunks, 2), 2);
  assert.equal(baseLineToTarget(hunks, 3), 2);
  assert.equal(baseLineToTarget(hunks, 15), 17);
});

void test("base lines between and after hunks shift by what the hunks before them changed", () => {
  assert.equal(baseLineToTarget(hunks, 4), 5);
  assert.equal(baseLineToTarget(hunks, 8), 10); // after the insertion at target line 9
  assert.equal(baseLineToTarget(hunks, 16), 17);
  assert.equal(baseLineToTarget([], 5), 5);
});

const at = (path: string, external: boolean): SemanticLocation => ({
  path,
  span: { start: { line: 0, col: 0 }, end: { line: 0, col: 1 } },
  external,
});

void test("a location inside the repository is preferred", () => {
  assert.equal(pickLocation([at("/dep.rs", true), at("a.rs", false)])?.path, "a.rs");
  assert.equal(pickLocation([at("/dep.rs", true)])?.path, "/dep.rs");
  assert.equal(pickLocation([]), undefined);
});
