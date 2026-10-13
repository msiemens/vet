import assert from "node:assert/strict";
import { test } from "node:test";
import type { Hunk } from "../../../generated/Hunk";
import type { HunkState } from "../../../generated/HunkState";
import type { FindMatch } from "./findMatches.ts";
import { TICK_ROWS, findTicks, markKind, sectionMarks } from "./scrollMarks.ts";

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

const METRICS_TICKS = { ...METRICS, header: 26, gap: 8 };
const at = (kind: FindMatch["kind"], line: number, order: number): FindMatch => ({
  kind,
  line,
  baseLine: line,
  hunk: kind === "context" ? -1 : 0,
  order,
  from: 0,
  to: 1,
});

void test("a find's tick of a changed line lies in its hunk's mark, in the order of the lines", () => {
  const section = {
    top: 100,
    height: 400,
    hunkTops: [30],
    file: { path: "a", hunks: [hunk(0, 2, 0, 3)], targetLines: 50 },
  };
  const [mark] = sectionMarks([section], () => undefined, 500, "unified", METRICS).marks;
  const found = [at("removed", 0, 0), at("removed", 1, 1), at("added", 0, 2), at("added", 2, 4)];
  const ticks = findTicks([section], () => found, undefined, "unified", METRICS_TICKS, 500);
  assert.equal(ticks.length, 4);
  for (const t of ticks) assert.ok(t.top >= mark.top && t.top <= mark.top + mark.height, `${t.top} in ${mark.top}`);
  assert.deepEqual(
    ticks.map((t) => t.top),
    ticks.map((t) => t.top).sort((a, b) => a - b),
  );
});

void test("a find's tick of an unchanged line lies in its section and moves down with the line", () => {
  const section = { top: 100, height: 400, hunkTops: [], file: { path: "a", hunks: [], targetLines: 50 } };
  const found = [at("context", 5, 5), at("context", 25, 25), at("context", 45, 45)];
  const ticks = findTicks([section], () => found, undefined, "unified", METRICS_TICKS, 500);
  const tops = ticks.map((t) => t.top);
  assert.deepEqual(
    tops,
    [...tops].sort((a, b) => a - b),
  );
  assert.ok(tops[0] > 100 && tops[2] < 500);
  assert.equal(new Set(tops).size, 3);
});

void test("a tick tells the current match from the others, and a split view's matches in one row share a tick", () => {
  const section = {
    top: 0,
    height: 400,
    hunkTops: [30],
    file: { path: "a", hunks: [hunk(0, 1, 0, 1)], targetLines: 50 },
  };
  const found = [at("removed", 0, 0), at("added", 0, 1)];
  const unified = findTicks([section], () => found, { section: 0, order: 1, from: 0 }, "unified", METRICS_TICKS, 400);
  assert.deepEqual(
    unified.map((t) => t.kind),
    ["find", "findCurrent"],
  );
  const split = findTicks([section], () => found, undefined, "split", METRICS_TICKS, 400);
  assert.equal(split.length, 1);
  assert.ok(unified[0].top < unified[1].top);
});

void test("a match on every line costs the scroll bar a bounded number of ticks, and the current match keeps its own", () => {
  const lines = 5000;
  const section = { top: 0, height: lines * 20, hunkTops: [], file: { path: "a", hunks: [], targetLines: lines } };
  const found = Array.from({ length: lines }, (_, i) => at("context", i, i));
  const total = lines * 20;
  const all = findTicks([section], () => found, undefined, "unified", METRICS_TICKS, total);
  assert.ok(all.length <= TICK_ROWS, `${all.length} ticks`);
  assert.ok(all.length > TICK_ROWS / 2);
  const withCurrent = findTicks(
    [section],
    () => found,
    { section: 0, order: 100, from: 0 },
    "unified",
    METRICS_TICKS,
    total,
  );
  assert.equal(withCurrent.filter((t) => t.kind === "findCurrent").length, 1);
  // Matches far apart do not share a tick.
  const apart = findTicks([section], () => [found[0], found[4999]], undefined, "unified", METRICS_TICKS, total);
  assert.equal(apart.length, 2);
});
