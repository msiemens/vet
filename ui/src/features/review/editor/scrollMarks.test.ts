import assert from "node:assert/strict";
import { test } from "node:test";
import type { Hunk } from "../../../generated/Hunk";
import type { HunkState } from "../../../generated/HunkState";
import { markKind, sectionMarks } from "./scrollMarks.ts";

const hunk = (baseStart: number, baseLen: number, targetStart: number, targetLen: number): Hunk => ({
  baseStart,
  baseLen,
  targetStart,
  targetLen,
});
const METRICS = { hunk: 24, line: 20 };

void test("a hunk's mark says what is left of it: removals show apart while unseen only", () => {
  assert.equal(markKind("unseen", false, false), "unseen");
  assert.equal(markKind("unseen", true, false), "deletion");
  assert.equal(markKind("seen", true, false), "seen");
  assert.equal(markKind("seenAgainstOldBase", true, false), "baseChanged");
  assert.equal(markKind("unapplied", false, false), "unseen");
  assert.equal(markKind("informational", false, false), "seen");
});

void test("a flag outranks every state in the mark", () => {
  for (const state of ["unseen", "seen", "seenAgainstOldBase", "unapplied", "informational"] as HunkState[])
    for (const deletion of [false, true]) assert.equal(markKind(state, deletion, true), "flagged");
});

void test("a diff view's marks lie where the height model puts its hunks", () => {
  const sections = [
    { top: 100, hunkTops: [30, 200], file: { path: "a", hunks: [hunk(0, 2, 0, 3), hunk(9, 4, 12, 0)] } },
    { top: 900, hunkTops: [30], file: { path: "b", hunks: [hunk(0, 1, 0, 1)] } },
  ];
  const states = new Map([
    ["a", { hunks: ["seen", "unseen"] as HunkState[], flagged: [false, false] }],
    ["b", { hunks: ["seenAgainstOldBase"] as HunkState[], flagged: [true] }],
  ]);
  const review = (path: string) => states.get(path);
  assert.deepEqual(sectionMarks(sections, review, 1200, "unified", METRICS), {
    total: 1200,
    marks: [
      { top: 130, height: 24 + 5 * 20, kind: "seen" },
      { top: 300, height: 24 + 4 * 20, kind: "deletion" },
      { top: 930, height: 24 + 2 * 20, kind: "flagged" },
    ],
  });
  // Split shows the taller side of a hunk only.
  const split = sectionMarks(sections, review, 1200, "split", METRICS).marks.map((m) => m.height);
  assert.deepEqual(split, [24 + 3 * 20, 24 + 4 * 20, 24 + 20]);
});
