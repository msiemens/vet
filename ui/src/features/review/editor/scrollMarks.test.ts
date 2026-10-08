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
  assert.equal(markKind("unseen", false), "unseen");
  assert.equal(markKind("unseen", true), "deletion");
  assert.equal(markKind("seen", true), "seen");
  assert.equal(markKind("seenAgainstOldBase", true), "baseChanged");
  assert.equal(markKind("unapplied", false), "unseen");
  assert.equal(markKind("informational", false), "seen");
});

void test("a diff view's marks lie where the height model puts its hunks", () => {
  const sections = [
    {
      top: 100,
      hunkTops: [30, 200],
      file: { hunks: [hunk(0, 2, 0, 3), hunk(9, 4, 12, 0)] },
      review: { hunks: ["seen", "unseen"] as HunkState[] },
    },
    {
      top: 900,
      hunkTops: [30],
      file: { hunks: [hunk(0, 1, 0, 1)] },
      review: { hunks: ["seenAgainstOldBase"] as HunkState[] },
    },
  ];
  assert.deepEqual(sectionMarks(sections, 1200, "unified", METRICS), {
    total: 1200,
    marks: [
      { top: 130, height: 24 + 5 * 20, kind: "seen" },
      { top: 300, height: 24 + 4 * 20, kind: "deletion" },
      { top: 930, height: 24 + 2 * 20, kind: "baseChanged" },
    ],
  });
  // Split shows the taller side of a hunk only.
  const split = sectionMarks(sections, 1200, "split", METRICS).marks.map((m) => m.height);
  assert.deepEqual(split, [24 + 3 * 20, 24 + 4 * 20, 24 + 20]);
});
