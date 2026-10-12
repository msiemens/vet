import assert from "node:assert/strict";
import { test } from "node:test";
import { stepFlagged } from "./flaggedHunks.ts";

const state = new Map<string, boolean[]>([
  ["a.ts", [false, true, false]],
  ["b.ts", [false]],
  ["c.ts", [true, false, true]],
]);
const paths = ["a.ts", "b.ts", "c.ts"];
const step = (from: Parameters<typeof stepFlagged>[2], delta: 1 | -1) =>
  stepFlagged(paths, (path) => state.get(path), from, delta);

void test("the next flagged hunk is found in the file, then in the files after it", () => {
  assert.deepEqual(step(undefined, 1), { path: "a.ts", hunk: 1 });
  assert.deepEqual(step({ path: "a.ts", hunk: 0 }, 1), { path: "a.ts", hunk: 1 });
  assert.deepEqual(step({ path: "a.ts", hunk: 1 }, 1), { path: "c.ts", hunk: 0 });
  assert.deepEqual(step({ path: "c.ts", hunk: 0 }, 1), { path: "c.ts", hunk: 2 });
  assert.equal(step({ path: "c.ts", hunk: 2 }, 1), undefined);
});

void test("the previous flagged hunk is found backwards across files, and none wraps around", () => {
  assert.deepEqual(step(undefined, -1), { path: "c.ts", hunk: 2 });
  assert.deepEqual(step({ path: "c.ts", hunk: 0 }, -1), { path: "a.ts", hunk: 1 });
  assert.equal(step({ path: "a.ts", hunk: 1 }, -1), undefined);
});

void test("a change card before a file's hunks counts as the place before its first hunk", () => {
  assert.deepEqual(step({ path: "c.ts", hunk: -1 }, 1), { path: "c.ts", hunk: 0 });
  assert.deepEqual(step({ path: "c.ts", hunk: -1 }, -1), { path: "a.ts", hunk: 1 });
});
