import assert from "node:assert/strict";
import { test } from "node:test";
import type { FileText, FindOptions } from "./findMatches.ts";
import { FindRun, MAX_SEARCH_CHARS } from "./findRun.ts";
import { editorMarks } from "./findView.ts";

const options = (text: string, over: Partial<FindOptions> = {}): FindOptions => ({
  text,
  matchCase: false,
  wholeWord: false,
  regex: false,
  side: "both",
  onlyChanges: false,
  ...over,
});

const plain = (target: string): FileText => ({ base: target, target, hunks: [] });

void test("the current match is the first one from where the search started", () => {
  const run = new FindRun(options("ab"), 2, "diff", { section: 1, order: 0, from: 0 });
  run.add(0, plain("ab\nab"));
  assert.deepEqual(run.current, { section: 0, order: 0, from: 0 });
  run.add(1, plain("x\nab"));
  // Matches found later in front of the current one do not move it.
  assert.deepEqual(run.current, { section: 0, order: 0, from: 0 });
  const s = run.summary();
  assert.equal(s.total, 3);
  assert.equal(s.index, 0);
  assert.equal(s.pending, 0);
});

void test("while sections are still being read the summary says so", () => {
  const run = new FindRun(options("ab"), 3, "diff");
  run.add(1, plain("ab"));
  assert.equal(run.summary().pending, 2);
  assert.equal(run.summary().total, 1);
});

void test("a query the engine rejects finds nothing and says why", () => {
  const run = new FindRun(options("(", { regex: true }), 1, "diff");
  run.add(0, plain("("));
  const s = run.summary();
  assert.equal(s.total, 0);
  assert.ok(s.error);
  assert.equal(s.pending, 0);
});

void test("a text too large to search is skipped and counted", () => {
  const run = new FindRun(options("ab"), 1, "diff");
  run.add(0, plain("ab".repeat(MAX_SEARCH_CHARS)));
  assert.equal(run.summary().total, 0);
  assert.equal(run.summary().skipped, 1);
});

void test("a side by side view marks a context line in both panes and dims the side not asked for", () => {
  const file: FileText = {
    base: "ab\nold ab",
    target: "ab\nnew ab",
    hunks: [{ baseStart: 1, baseLen: 1, targetStart: 1, targetLen: 1 }],
  };
  const run = new FindRun(options("ab", { side: "target" }), 1, "diff");
  run.add(0, file);
  const marks = editorMarks(0, run.matchesOf(0), run.current, "split", "target");
  assert.deepEqual(
    marks.target.ranges.map((r) => r.line),
    [0, 1],
  );
  assert.deepEqual(
    marks.base.ranges.map((r) => r.line),
    [0],
  );
  assert.equal(marks.base.dim, true);
  assert.equal(marks.target.dim, false);
  assert.deepEqual(
    marks.target.ranges.map((r) => r.current),
    [true, false],
  );
  assert.deepEqual(
    marks.base.ranges.map((r) => r.current),
    [true],
  );
});

void test("a unified view marks a removed line in the deleted lines, by base line", () => {
  const file: FileText = {
    base: "x\nold ab",
    target: "x\nnew",
    hunks: [{ baseStart: 1, baseLen: 1, targetStart: 1, targetLen: 1 }],
  };
  const run = new FindRun(options("ab"), 1, "diff");
  run.add(0, file);
  const marks = editorMarks(0, run.matchesOf(0), run.current, "unified", "both");
  assert.deepEqual(marks.target.ranges, []);
  assert.deepEqual(
    marks.target.removed.map((r) => [r.line, r.from, r.to, r.current]),
    [[1, 4, 6, true]],
  );
});
