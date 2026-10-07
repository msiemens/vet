import assert from "node:assert/strict";
import { test } from "node:test";
import type { ChangesetRow } from "../../generated/ChangesetRow";
import type { Completion } from "../../generated/Completion";
import { filterRows, reviewedOf, sourceText, tabCounts, updatedText } from "./queueRows.ts";

const NOW = new Date(2026, 9, 7, 15, 30).getTime();
const tree: ChangesetRow = {
  key: { type: "workingTree" },
  title: "Working tree",
  checkedOut: false,
  branch: "main",
  openThreads: 0,
  review: { state: "none" },
};
const retry: ChangesetRow = {
  key: { type: "branch", name: "feat/retry-policy" },
  title: "feat/retry-policy",
  checkedOut: false,
  openThreads: 2,
  divergence: { ahead: 3, behind: 0 },
  lastCommit: NOW / 1000 - 3 * 3600,
  review: { state: "open", version: 4, startedAt: 0 },
};
const skew: ChangesetRow = {
  key: { type: "branch", name: "fix/clock-skew" },
  title: "fix/clock-skew",
  checkedOut: false,
  openThreads: 0,
  divergence: { ahead: 1, behind: 2 },
  lastCommit: NOW / 1000 - 2 * 86400,
  review: { state: "finished", complete: true, startedAt: 0, finishedAt: 0 },
};
const rows = [tree, retry, skew];

void test("the source says which branch the working tree is on and how many commits a branch has", () => {
  assert.equal(sourceText(tree), "main");
  assert.equal(sourceText(retry), "local · 3 commits");
  assert.equal(sourceText(skew), "local · 1 commit");
  assert.equal(sourceText({ ...skew, divergence: undefined }), "local");
});

void test("the working tree is updated now and a branch with its last commit", () => {
  assert.equal(updatedText(tree, NOW), "now");
  assert.equal(updatedText(retry, NOW), "12:30 PM");
  assert.equal(updatedText({ ...retry, lastCommit: undefined }, NOW), "");
});

void test("a tab lists its rows in order and the search narrows them by title and source", () => {
  const titles = (tab: Parameters<typeof filterRows>[1], query = "") =>
    filterRows(rows, tab, query).map((r) => r.title);
  assert.deepEqual(titles("all"), ["Working tree", "feat/retry-policy", "fix/clock-skew"]);
  assert.deepEqual(titles("prs"), []);
  assert.deepEqual(titles("all", "RETRY"), ["feat/retry-policy"]);
  assert.deepEqual(titles("local", "local 3"), ["feat/retry-policy"]);
  assert.deepEqual(titles("all", "main"), ["Working tree"]);
});

void test("the counts follow the tabs", () => {
  assert.deepEqual(tabCounts(rows), { all: 3, blocking: 0, local: 3, prs: 0 });
  assert.deepEqual(tabCounts([]), { all: 0, blocking: 0, local: 0, prs: 0 });
});

void test("the progress column follows the review, and shows progress only where it is known", () => {
  const completion = { seenPercent: 61, oldBasePercent: 4 } as Completion;
  assert.deepEqual(reviewedOf(tree.review), { kind: "none" });
  assert.deepEqual(reviewedOf(retry.review, completion), { kind: "progress", seen: 61, oldBase: 4 });
  assert.deepEqual(reviewedOf(retry.review), { kind: "started" });
  assert.deepEqual(reviewedOf(skew.review), { kind: "finished", percent: 100 });
  assert.deepEqual(reviewedOf({ state: "finished", complete: false, startedAt: 0, finishedAt: 0 }, completion), {
    kind: "finished",
  });
});
