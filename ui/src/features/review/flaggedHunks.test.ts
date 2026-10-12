import assert from "node:assert/strict";
import { test } from "node:test";
import { type FlagFile, stepFlagged } from "./flaggedHunks.ts";

const file = (path: string, ...flagged: boolean[]): FlagFile => ({ path, review: { flagged } });
const files = [file("a.ts", false, true, false), file("b.ts", false), file("c.ts", true, false, true)];

void test("the next flagged hunk is found in the file, then in the files after it", () => {
  assert.deepEqual(stepFlagged(files, undefined, 1), { path: "a.ts", hunk: 1 });
  assert.deepEqual(stepFlagged(files, { path: "a.ts", hunk: 0 }, 1), { path: "a.ts", hunk: 1 });
  assert.deepEqual(stepFlagged(files, { path: "a.ts", hunk: 1 }, 1), { path: "c.ts", hunk: 0 });
  assert.deepEqual(stepFlagged(files, { path: "c.ts", hunk: 0 }, 1), { path: "c.ts", hunk: 2 });
  assert.equal(stepFlagged(files, { path: "c.ts", hunk: 2 }, 1), undefined);
});

void test("the previous flagged hunk is found backwards across files, and none wraps around", () => {
  assert.deepEqual(stepFlagged(files, undefined, -1), { path: "c.ts", hunk: 2 });
  assert.deepEqual(stepFlagged(files, { path: "c.ts", hunk: 0 }, -1), { path: "a.ts", hunk: 1 });
  assert.equal(stepFlagged(files, { path: "a.ts", hunk: 1 }, -1), undefined);
});

void test("a change card before a file's hunks counts as the place before its first hunk", () => {
  assert.deepEqual(stepFlagged(files, { path: "c.ts", hunk: -1 }, 1), { path: "c.ts", hunk: 0 });
  assert.deepEqual(stepFlagged(files, { path: "c.ts", hunk: -1 }, -1), { path: "a.ts", hunk: 1 });
});
