import assert from "node:assert/strict";
import { test } from "node:test";
import type { Side } from "../../../generated/Side.ts";
import { type EditorSelection, fileViewComment, hunkComment, linesAt } from "./commentAnchor.ts";
import type { LineDoc } from "./positions.ts";

/** Ten lines of nine characters and a break: line `n` starts at offset 10n. */
const doc: LineDoc = {
  lines: 10,
  line: (n) => ({ from: (n - 1) * 10, to: (n - 1) * 10 + 9 }),
  lineAt: (pos) => ({ number: Math.floor(pos / 10) + 1 }),
};

/** A selection from line `a` column `ca` to line `b` column `cb`. */
const sel = (a: number, ca: number, b = a, cb = ca, side: Side = "target"): EditorSelection => ({
  side,
  doc,
  from: a * 10 + ca,
  to: b * 10 + cb,
});

// Target lines 2..4 replaced base lines 2..3; base lines 7..8 were removed before target line 8.
const hunks = [
  { baseStart: 2, baseLen: 2, targetStart: 2, targetLen: 3 },
  { baseStart: 7, baseLen: 2, targetStart: 8, targetLen: 0 },
];

void test("a comment goes on the selected lines, else on the caret's line, on the editor's side", () => {
  assert.deepEqual(linesAt(sel(3, 4, 5, 2)), { type: "lines", side: "target", start: 3, len: 3 });
  assert.deepEqual(linesAt(sel(6, 3)), { type: "lines", side: "target", start: 6, len: 1 });
  assert.deepEqual(linesAt(sel(1, 0, 1, 0, "base")), { type: "lines", side: "base", start: 1, len: 1 });
  // Whole lines selected down to the start of the next one.
  assert.deepEqual(linesAt(sel(3, 0, 5, 0)), { type: "lines", side: "target", start: 3, len: 2 });
});

void test("a hunk's button comments on the selection inside the hunk", () => {
  assert.deepEqual(hunkComment(hunks, 0, sel(3, 1, 3, 5)), { type: "lines", side: "target", start: 3, len: 1 });
  // Overlapping the hunk is enough: the selection is taken as it is.
  assert.deepEqual(hunkComment(hunks, 0, sel(0, 2, 2, 1)), { type: "lines", side: "target", start: 0, len: 3 });
  assert.deepEqual(hunkComment(hunks, 1, sel(7, 0, 8, 3, "base")), {
    type: "lines",
    side: "base",
    start: 7,
    len: 2,
  });
});

void test("a hunk's button comments on the whole hunk for a caret, or a selection outside it", () => {
  const whole = { type: "hunk", index: 0 };
  assert.deepEqual(hunkComment(hunks, 0), whole);
  assert.deepEqual(hunkComment(hunks, 0, sel(3, 4)), whole);
  assert.deepEqual(hunkComment(hunks, 0, sel(5, 0, 6, 4)), whole);
  // Ending at the start of the hunk's first line does not reach into it.
  assert.deepEqual(hunkComment(hunks, 0, sel(1, 0, 2, 0)), whole);
  // The same lines on the other side are other text.
  assert.deepEqual(hunkComment(hunks, 0, sel(4, 0, 4, 5, "base")), whole);
  // A removal has no target lines for a selection to lie in.
  assert.deepEqual(hunkComment(hunks, 1, sel(7, 0, 8, 5)), { type: "hunk", index: 1 });
});

void test("in a file's text a comment goes on the selection or the caret's line", () => {
  assert.deepEqual(fileViewComment(hunks, 0, sel(6, 0, 7, 3)), { type: "lines", side: "target", start: 6, len: 2 });
  assert.deepEqual(fileViewComment(hunks, -1, sel(6, 3)), { type: "lines", side: "target", start: 6, len: 1 });
  assert.deepEqual(fileViewComment(hunks, 0, sel(3, 3)), { type: "lines", side: "target", start: 3, len: 1 });
});

void test("in a file's text a caret at a removal, or no editor entered, comments on the hunk", () => {
  assert.deepEqual(fileViewComment(hunks, 1, sel(8, 0)), { type: "hunk", index: 1 });
  assert.deepEqual(fileViewComment(hunks, 0), { type: "hunk", index: 0 });
  assert.equal(fileViewComment(hunks, -1), undefined);
  // A selection made after choosing the removal is what the reviewer means.
  assert.deepEqual(fileViewComment(hunks, 1, sel(8, 0, 9, 2)), { type: "lines", side: "target", start: 8, len: 2 });
});
