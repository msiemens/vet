import assert from "node:assert/strict";
import { test } from "node:test";
import { compile } from "../../../lib/search/query.ts";
import {
  type FileText,
  type FindMatch,
  type FindOptions,
  type SectionMatches,
  findInFile,
  indexOf,
  nearest,
  orderOf,
  posOf,
  step,
} from "./findMatches.ts";

const OPTIONS: FindOptions = {
  text: "delay",
  matchCase: false,
  wholeWord: false,
  regex: false,
  side: "both",
  onlyChanges: false,
};

/** Line 2 of the base ("old delay") is replaced by two lines; "delay" also stands in context lines 0 and 4. */
const FILE: FileText = {
  base: "delay a\nkeep\nold delay\nkeep\ndelay z",
  target: "delay a\nkeep\nnew delay\nnext delay\nkeep\ndelay z",
  hunks: [{ baseStart: 2, baseLen: 1, targetStart: 2, targetLen: 2 }],
};

function run(over: Partial<FindOptions> = {}, mode: "diff" | "file" = "diff", file = FILE): FindMatch[] {
  const options = { ...OPTIONS, ...over };
  const c = compile(options);
  if (c.type !== "ok") throw new Error(`the query did not compile: ${c.type}`);
  return findInFile(file, c.re, options, mode);
}

/** The match at `index` of a list. */
function nth(list: readonly FindMatch[], index: number): FindMatch {
  const match = list.at(index);
  if (!match) throw new Error(`no match at ${index}`);
  return match;
}

const shape = (ms: FindMatch[]) => ms.map((m) => `${m.kind}:${m.line}`);

void test("a diff lists context once and a hunk's removed lines before its added ones", () => {
  assert.deepEqual(shape(run()), ["context:0", "removed:2", "added:2", "added:3", "context:5"]);
});

void test("a context line carries the base line it stands for", () => {
  const last = nth(run(), -1);
  assert.equal(last.line, 5);
  assert.equal(last.baseLine, 4);
});

void test("the side narrows a diff, and a context line stays on both", () => {
  assert.deepEqual(shape(run({ side: "target" })), ["context:0", "added:2", "added:3", "context:5"]);
  assert.deepEqual(shape(run({ side: "base" })), ["context:0", "removed:2", "context:5"]);
});

void test("only changes drops the context lines", () => {
  assert.deepEqual(shape(run({ onlyChanges: true })), ["removed:2", "added:2", "added:3"]);
});

void test("the File view searches the target's text and ignores the side", () => {
  assert.deepEqual(shape(run({ side: "base" }, "file")), ["context:0", "added:2", "added:3", "context:5"]);
  assert.deepEqual(shape(run({ onlyChanges: true }, "file")), ["added:2", "added:3"]);
});

void test("a file without hunks is all context", () => {
  const same: FileText = { base: "x delay\ny", target: "x delay\ny", hunks: [] };
  assert.deepEqual(shape(run({}, "diff", same)), ["context:0"]);
});

void test("a deletion at the end and an insertion at the start line up with their lines", () => {
  const file: FileText = {
    base: "a\ndelay b\nc",
    target: "delay new\na\nc",
    hunks: [
      { baseStart: 0, baseLen: 0, targetStart: 0, targetLen: 1 },
      { baseStart: 1, baseLen: 1, targetStart: 2, targetLen: 0 },
    ],
  };
  assert.deepEqual(shape(run({}, "diff", file)), ["added:0", "removed:1"]);
});

void test("stepping goes on through the sections and says when it wraps", () => {
  const [a, b] = [run(), run()];
  const all: SectionMatches = [a, b];
  const last = posOf(1, nth(b, -1));
  assert.deepEqual(step(all, last, 1), { to: posOf(0, nth(a, 0)), wrapped: true });
  assert.deepEqual(step(all, posOf(0, nth(a, -1)), 1), { to: posOf(1, nth(b, 0)), wrapped: false });
  assert.deepEqual(step(all, posOf(0, nth(a, 0)), -1), { to: last, wrapped: true });
  assert.equal(step([[], []], undefined, 1), undefined);
});

void test("the place a step started from may be no match any more", () => {
  const list = run();
  const all: SectionMatches = [list];
  // Between the second and third match.
  const gone = { section: 0, order: nth(list, 1).order, from: nth(list, 1).from + 1 };
  assert.deepEqual(step(all, gone, 1), { to: posOf(0, nth(list, 2)), wrapped: false });
  assert.deepEqual(step(all, gone, -1), { to: posOf(0, nth(list, 1)), wrapped: false });
  assert.deepEqual(nearest(all, gone), posOf(0, nth(list, 2)));
});

void test("a section not searched yet adds nothing to the count before a place", () => {
  const list = run();
  const all: SectionMatches = [undefined, list];
  assert.equal(indexOf(all, posOf(1, nth(list, 2))), 2);
});

void test("the place of a line in the order is the one the search gave its matches", () => {
  for (const mode of ["diff", "file"] as const) {
    // "e" is in every line, so every kind of line has a match.
    const list = run({ text: "e" }, mode);
    assert.ok(list.length > 5);
    for (const m of list)
      assert.equal(orderOf(FILE.hunks, m.kind, m.line, mode), m.order, `${mode} ${m.kind}:${m.line}`);
  }
});
