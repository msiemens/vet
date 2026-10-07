import assert from "node:assert/strict";
import { test } from "node:test";
import { ChangeSet } from "@codemirror/state";
import { NO_HIGHLIGHTS, decodeHighlights, eachRange, mapHighlights, pieces } from "./highlights.ts";

/** `let a = 1;`: `let` keyword at 0..3, `1` number at 8..9. */
const wire = [0, 3, 0, 5, 1, 2];

const ranges = (h = decodeHighlights(wire), from = 0, to = 100) => {
  const out: [number, number, string][] = [];
  eachRange(h, from, to, (a, b, c) => out.push([a, b, c]));
  return out;
};

void test("the gaps and lengths decode into absolute ranges", () => {
  const h = decodeHighlights(wire);
  assert.deepEqual([...h.from], [0, 8]);
  assert.deepEqual([...h.to], [3, 9]);
  assert.deepEqual([...h.kind], [0, 2]);
  assert.equal(decodeHighlights([]), NO_HIGHLIGHTS);
});

void test("ranges become classes by kind", () => {
  assert.deepEqual(ranges(), [
    [0, 3, "syn-keyword"],
    [8, 9, "syn-number"],
  ]);
  const kinds = decodeHighlights([0, 1, 0, 0, 1, 1, 0, 1, 2, 0, 1, 3, 0, 1, 4, 0, 1, 5, 0, 1, 6, 0, 1, 7, 0, 1, 8]);
  assert.deepEqual(
    ranges(kinds).map((r) => r[2]),
    [
      "syn-keyword",
      "syn-string",
      "syn-number",
      "syn-comment",
      "syn-type",
      "syn-function",
      "syn-attribute",
      "syn-heading",
      "syn-link",
    ],
  );
});

void test("only the ranges in view are produced, cut to it", () => {
  assert.deepEqual(ranges(undefined, 1, 8), [[1, 3, "syn-keyword"]]);
  assert.deepEqual(ranges(undefined, 3, 8), []);
  assert.deepEqual(ranges(undefined, 8, 20), [[8, 9, "syn-number"]]);
  assert.deepEqual(ranges(NO_HIGHLIGHTS), []);
});

void test("a kind the UI does not know is skipped", () => {
  assert.deepEqual(ranges(decodeHighlights([0, 2, 99, 1, 2, 0])), [[3, 5, "syn-keyword"]]);
});

void test("text is cut into plain and coloured pieces", () => {
  assert.deepEqual(pieces(decodeHighlights(wire), 0, "let a = 1;"), [
    { text: "let", className: "syn-keyword" },
    " a = ",
    { text: "1", className: "syn-number" },
    ";",
  ]);
  // A text that starts at an offset of the document.
  assert.deepEqual(pieces(decodeHighlights(wire), 7, " 1;"), [" ", { text: "1", className: "syn-number" }, ";"]);
  assert.deepEqual(pieces(NO_HIGHLIGHTS, 0, "plain"), ["plain"]);
});

void test("edits move the ranges and drop what they remove", () => {
  const h = decodeHighlights(wire);
  // Two characters inserted at the start push everything right.
  let changes = ChangeSet.of({ from: 0, insert: "xx" }, 10);
  let mapped = mapHighlights(h, (pos, assoc) => changes.mapPos(pos, assoc));
  assert.deepEqual([...mapped.from, ...mapped.to], [2, 10, 5, 11]);
  // Typing inside a range keeps it whole; typing at its end does not extend it.
  changes = ChangeSet.of(
    [
      { from: 1, insert: "ab" },
      { from: 9, insert: "!" },
    ],
    10,
  );
  mapped = mapHighlights(h, (pos, assoc) => changes.mapPos(pos, assoc));
  assert.deepEqual([...mapped.from, ...mapped.to], [0, 10, 5, 11]);
  // Deleting a whole range removes it.
  changes = ChangeSet.of({ from: 8, to: 9 }, 10);
  mapped = mapHighlights(h, (pos, assoc) => changes.mapPos(pos, assoc));
  assert.deepEqual([...mapped.from], [0]);
  assert.deepEqual([...mapped.kind], [0]);
});
