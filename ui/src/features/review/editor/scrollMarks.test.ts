import assert from "node:assert/strict";
import { test } from "node:test";
import type { Hunk } from "../../../generated/Hunk";
import type { HunkState } from "../../../generated/HunkState";
import { MIN_MARK, bandRect, markKind, markRects, sectionMarks } from "./scrollMarks.ts";

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

void test("marks are scaled to the track and keep to it", () => {
  const rects = markRects(
    {
      total: 1000,
      marks: [
        { top: 0, height: 100, kind: "seen" },
        { top: 500, height: 200, kind: "unseen" },
        { top: 990, height: 100, kind: "deletion" },
      ],
    },
    100,
  );
  assert.deepEqual(rects, [
    { top: 0, height: 10, kind: "seen" },
    { top: 50, height: 20, kind: "unseen" },
    { top: 90, height: 10, kind: "deletion" },
  ]);
});

void test("a hunk too small for the track still gets a mark you can see", () => {
  const [rect] = markRects({ total: 100_000, marks: [{ top: 50_000, height: 20, kind: "unseen" }] }, 500);
  assert.equal(rect.height, MIN_MARK);
  assert.equal(rect.top, 250);
});

void test("nothing is drawn without content or a track", () => {
  const marks = [{ top: 0, height: 10, kind: "seen" as const }];
  assert.deepEqual(markRects({ total: 0, marks }, 100), []);
  assert.deepEqual(markRects({ total: 100, marks }, 0), []);
});

void test("the band is the part in view, and stays on the track at both ends", () => {
  assert.deepEqual(bandRect(1000, 250, 100, 200), { top: 50, height: 20 });
  assert.deepEqual(bandRect(1000, 0, 100, 200), { top: 0, height: 20 });
  assert.deepEqual(bandRect(1000, 950, 100, 200), { top: 180, height: 20 });
  assert.deepEqual(bandRect(1000, -40, 100, 200), { top: 0, height: 20 });
  assert.equal(bandRect(80, 0, 100, 200).height, 200, "content that fits is all in view");
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
