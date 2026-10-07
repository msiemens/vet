import assert from "node:assert/strict";
import { test } from "node:test";
import type { FileChange } from "../../../generated/FileChange.ts";
import type { NonTextItem } from "../../../generated/NonTextItem.ts";
import { hiddenAt, sectionLayout, shownLines } from "./sectionLayout.ts";

const metrics = { line: 20, hunk: 24, header: 26, collapsed: 27, gap: 8, preview: 100 };
const hunk = (targetStart: number, baseLen: number, targetLen: number) => ({
  baseStart: targetStart,
  baseLen,
  targetStart,
  targetLen,
});
const file = (targetLines: number, hunks: ReturnType<typeof hunk>[], nonText: NonTextItem[] = [], path = "a.rs") =>
  ({ path, targetLines, hunks, nonText }) as FileChange;
const layout = (f: FileChange, mode: "unified" | "split", collapse = true) => sectionLayout(f, mode, metrics, collapse);
const entry = { kind: "file", mode: 0o100644, target: null, size: 1 } as const;
const item = (kind: NonTextItem["kind"], hasPreview = false): NonTextItem => ({
  kind,
  before: entry,
  after: entry,
  hasPreview,
  modeChange: null,
});

void test("unchanged regions collapse to one bar each, keeping 3 lines of context", () => {
  const f = file(100, [hunk(50, 2, 3)]);
  // header, 3 lines + bar, hunk bar, 2 deleted + 3 added lines, 3 lines + bar, gap.
  assert.deepEqual(layout(f, "unified"), {
    height: 26 + 87 + 24 + 100 + 87 + 8,
    cardTops: [],
    cardsHeight: 0,
    hunkTops: [113],
    drawn: [],
  });
  // Side by side pads the shorter side: 3 rows instead of 5.
  assert.equal(layout(f, "split").height, 26 + 87 + 24 + 60 + 87 + 8);
});

void test("side by side, a hunk that deletes lines is one bar taller: the filler counts the base side's bar", () => {
  const deleted = file(100, [hunk(50, 2, 0)]);
  assert.equal(layout(deleted, "split").height, 26 + 87 + 24 + 24 + 40 + 87 + 8);
  assert.equal(layout(deleted, "unified").height, 26 + 87 + 24 + 40 + 87 + 8);
  // A hunk that adds lines has no bar on the base side.
  assert.equal(layout(file(100, [hunk(50, 0, 3)]), "split").height, 26 + 87 + 24 + 60 + 87 + 8);
});

void test("a gap too small to hide lines stays open", () => {
  const l = layout(file(30, [hunk(10, 1, 1), hunk(17, 1, 1)]), "unified");
  assert.deepEqual(l.hunkTops, [113, 297]);
  assert.equal(l.height, 456);
});

void test("a file without hunks is one bar, or just the header when empty, plus the gap", () => {
  assert.equal(layout(file(40, []), "unified").height, 26 + 27 + 8);
  assert.equal(layout(file(0, []), "unified").height, 26 + 8);
});

void test("a hunk that ends at the last line is followed by the gap alone", () => {
  assert.equal(layout(file(7, [hunk(0, 0, 7)]), "unified").height, 26 + 24 + 7 * 20 + 8);
});

void test("without collapsing every line is a row", () => {
  const l = layout(file(100, [hunk(50, 2, 3)]), "unified", false);
  // header, 100 target lines, 2 deleted lines, one hunk bar; the bar sits after 50 lines.
  assert.deepEqual(l.hunkTops, [26 + 50 * 20]);
  assert.equal(l.height, 26 + 100 * 20 + 2 * 20 + 24 + 8);
});

void test("cards sit between the header and the first hunk and push everything down", () => {
  const cards = [item("mode"), { ...item("contentUnavailable"), before: null, after: null }];
  const plain = layout(file(100, [hunk(50, 2, 3)]), "unified");
  const l = layout(file(100, [hunk(50, 2, 3)], cards), "unified");
  // A card is a bar and two rows; a card with nothing to compare is a bar and one row.
  assert.deepEqual(l.cardTops, [26, 26 + 64]);
  assert.equal(l.cardsHeight, 64 + 44);
  assert.deepEqual(l.hunkTops, [113 + 108]);
  assert.equal(l.height, plain.height + 108);
});

void test("a file's thread block pushes everything down, an inline card only what is below it", () => {
  const f = file(100, [hunk(50, 2, 3)]);
  const plain = layout(f, "unified");
  const l = sectionLayout(f, "unified", metrics, true, {
    block: 50,
    inline: [
      { line: 51, height: 70 }, // in the hunk, above nothing but the bottom context
      { line: 10, height: 30 }, // in a collapsed region: not drawn
      { line: 48, height: 40 }, // in the context above the hunk, so the hunk bar is below it
    ],
  });
  assert.deepEqual(l.drawn, [true, false, true]);
  assert.equal(l.height, plain.height + 50 + 70 + 40);
  assert.deepEqual(l.hunkTops, [113 + 50 + 40]);
});

void test("shown lines are the hunks and their context, or everything without collapsing", () => {
  const f = file(100, [hunk(50, 2, 3)]);
  assert.deepEqual(shownLines(f, true), [
    [47, 50],
    [50, 53],
    [53, 56],
  ]);
  assert.deepEqual(shownLines(f, false), [[0, 100]]);
  // Two hunks close together leave one open gap.
  assert.deepEqual(shownLines(file(30, [hunk(10, 1, 1), hunk(17, 1, 1)]), true), [
    [7, 10],
    [10, 11],
    [11, 17],
    [17, 18],
    [18, 21],
  ]);
  assert.deepEqual(shownLines(file(40, []), true), []);
});

void test("a bar knows how many lines it hides, on either side", () => {
  const f = { ...file(100, [hunk(50, 2, 3)]), baseLines: 99 };
  assert.equal(hiddenAt(f, true, "target", 0), 47);
  assert.equal(hiddenAt(f, true, "target", 56), 44);
  assert.equal(hiddenAt(f, true, "target", 48), 0);
  // The base side counts its own lines: the hunk spans 2 of them, so the tail starts at 52.
  assert.equal(hiddenAt(f, true, "base", 55), 99 - 55);
  assert.equal(hiddenAt(f, false, "target", 0), 0);
  assert.equal(hiddenAt(file(40, []), true, "target", 5), 40);
});

void test("a card the backend says has a preview gets a preview strip", () => {
  assert.equal(layout(file(0, [], [item("binary", true)]), "unified").cardsHeight, 24 + 40 + 100);
  assert.equal(layout(file(0, [], [item("binary")]), "unified").cardsHeight, 24 + 40);
});
