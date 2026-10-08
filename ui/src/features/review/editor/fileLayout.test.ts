import assert from "node:assert/strict";
import { test } from "node:test";
import { Text } from "@codemirror/state";
import type { Hunk } from "../../../generated/Hunk";
import type { TextChange } from "../../../generated/TextChange";
import { fileLayout, hunkAtLine, oldLinesOf, removedLines, segments, stepHunk, tintOf, wedgeAt } from "./fileLayout.ts";

const hunk = (baseStart: number, baseLen: number, targetStart: number, targetLen: number): Hunk => ({
  baseStart,
  baseLen,
  targetStart,
  targetLen,
});
const change = (fromBase: number, toBase: number, fromTarget: number, toTarget: number): TextChange => ({
  fromBase,
  toBase,
  fromTarget,
  toTarget,
});
const doc = (text: string) => Text.of(text.split("\n"));

void test("added and modified lines belong to their hunk, a removal to none", () => {
  const target = doc("a\nb\nc\nd\ne\nf");
  const layout = fileLayout([hunk(1, 1, 1, 2), hunk(4, 2, 4, 0), hunk(6, 0, 5, 1)], [], target);
  assert.deepEqual(
    [...layout.lineHunk],
    [
      [1, 0],
      [2, 0],
      [5, 2],
    ],
  );
  assert.equal(hunkAtLine(layout, 4), undefined, "the line below a removal is not in its hunk");
  assert.equal(hunkAtLine(layout, 2), 0);
});

void test("a removal is anchored above the line that follows it, or below the last line", () => {
  const target = doc("a\nb\nc");
  const layout = fileLayout([hunk(1, 2, 1, 0), hunk(5, 1, 3, 0)], [], target);
  assert.deepEqual(layout.deletions, [
    { hunk: 0, line: 1 },
    { hunk: 1, line: 3 },
  ]);
});

void test("a removal's wedge sits on the edge of the line above it, or on the first line's top", () => {
  assert.deepEqual(wedgeAt({ hunk: 0, line: 4 }, 10), { line: 3, at: "below" });
  assert.deepEqual(wedgeAt({ hunk: 0, line: 0 }, 10), { line: 0, at: "above" });
  assert.deepEqual(wedgeAt({ hunk: 0, line: 10 }, 10), { line: 9, at: "below" });
});

void test("words are told apart on modified lines only, never on a whole line or an added one", () => {
  const target = doc("let exp = new(x);\nadded line\nwhole");
  const words = (changes: TextChange[], hunks: Hunk[]) =>
    fileLayout(hunks, changes, target).words.map((w) => target.sliceString(w.from, w.to));
  // The first line is modified: its changed word is marked.
  assert.deepEqual(words([change(10, 13, 10, 13)], [hunk(0, 1, 0, 1), hunk(1, 0, 1, 1)]), ["new"]);
  // The added line's change covers it whole, and so is no word.
  assert.deepEqual(words([change(0, 0, 18, 28)], [hunk(0, 1, 0, 1), hunk(1, 0, 1, 1)]), []);
  // Words of a hunk that only adds lines are not marked even where they are not whole lines.
  assert.deepEqual(words([change(0, 0, 22, 26)], [hunk(1, 0, 1, 1)]), []);
  // A change over a whole modified line leaves the line's own tint to mark it.
  assert.deepEqual(words([change(0, 17, 0, 17)], [hunk(0, 1, 0, 1)]), []);
});

void test("a change across lines is cut at the line break", () => {
  const target = doc("ab cd\nef gh\nij");
  const layout = fileLayout([hunk(0, 3, 0, 3)], [change(3, 9, 3, 9)], target);
  assert.deepEqual(
    layout.words.map((w) => target.sliceString(w.from, w.to)),
    ["cd", "ef "],
  );
});

void test("a tint is faint for what was seen and full for what is left", () => {
  assert.equal(tintOf("unseen"), "full");
  assert.equal(tintOf("seenAgainstOldBase"), "full");
  assert.equal(tintOf("unapplied"), "full");
  assert.equal(tintOf("seen"), "faint");
  assert.equal(tintOf("informational"), "faint");
});

void test("navigation from no hunk starts at the caret and stops at the ends", () => {
  const hunks = [hunk(2, 1, 2, 1), hunk(10, 1, 10, 1), hunk(20, 1, 20, 1)];
  assert.equal(stepHunk(hunks, -1, 5, 1), 1);
  assert.equal(stepHunk(hunks, -1, 5, -1), 0);
  assert.equal(stepHunk(hunks, -1, 30, 1), 2);
  assert.equal(stepHunk(hunks, -1, 0, -1), 0);
  assert.equal(stepHunk(hunks, 1, 12, 1), 2);
  assert.equal(stepHunk(hunks, 2, 20, 1), 2);
  assert.equal(stepHunk(hunks, 0, 2, -1), 0);
  assert.equal(stepHunk([], -1, 0, 1), -1);
});

void test("the old line of a modified line is its own, with the replaced words as columns", () => {
  const base = doc("keep\nlet exp = old(x);\nlet n = 1;");
  const h = hunk(1, 2, 1, 2);
  // "old" sits at base offsets 15..18 of the document, columns 10..13 of its line.
  const lines = oldLinesOf(h, 1, [change(15, 18, 15, 18)], base);
  assert.deepEqual(lines, [{ line: 1, text: "let exp = old(x);", marks: [{ from: 10, to: 13 }] }]);
  assert.deepEqual(oldLinesOf(h, 2, [change(15, 18, 15, 18)], base), [{ line: 2, text: "let n = 1;", marks: [] }]);
});

void test("a hunk that replaced another number of lines offers every old line; an added line has none", () => {
  const base = doc("a\nb\nc\nd");
  assert.deepEqual(
    oldLinesOf(hunk(1, 2, 1, 1), 1, [], base).map((l) => l.text),
    ["b", "c"],
  );
  assert.deepEqual(oldLinesOf(hunk(1, 0, 1, 2), 1, [], base), []);
});

void test("replaced words are cut out of the line", () => {
  assert.deepEqual(segments("let exp = old(x);", [{ from: 10, to: 13 }]), [
    { text: "let exp = ", marked: false },
    { text: "old", marked: true },
    { text: "(x);", marked: false },
  ]);
  assert.deepEqual(segments("abc", []), [{ text: "abc", marked: false }]);
});

void test("the lines a removal took are numbered as the base numbers them", () => {
  const base = doc("a\nb\nc\nd");
  assert.deepEqual(removedLines(hunk(1, 2, 1, 0), base), [
    { number: 2, text: "b" },
    { number: 3, text: "c" },
  ]);
});
