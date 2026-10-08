import assert from "node:assert/strict";
import { test } from "node:test";
import type { ChangesetRow } from "../../generated/ChangesetRow";
import type { Completion } from "../../generated/Completion";
import type { PullRequestRow } from "../../generated/PullRequestRow";
import type { Target } from "../../generated/Target";
import {
  commentCount,
  filterRows,
  labelOf,
  pullRequestNumber,
  pullRequestRow,
  queueRows,
  reviewedOf,
  shownLines,
  showsOwnLines,
  sourceText,
  tabCounts,
  targetLabel,
  updatedText,
} from "./queueRows.ts";

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

void test("a branch with no commits of its own shows no lines, whatever base the review picked", () => {
  assert.equal(showsOwnLines({ ...retry, divergence: { ahead: 0, behind: 5 } }), false);
  assert.equal(showsOwnLines(retry), true);
  assert.equal(showsOwnLines(tree), true);
});

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

const pr: PullRequestRow = {
  key: { type: "pullRequest", remote: "github.com/acme/app", number: 482 },
  title: "Add idempotency keys to sync jobs",
  author: "lena",
  sourceBranch: "lena/idempotency-keys",
  targetBranch: "main",
  lines: { added: 236, removed: 41 },
  comments: 2,
  ci: "success",
  draft: false,
  waitingForMe: true,
  unresolvedThreads: 1,
  updatedAt: NOW / 1000 - 3 * 3600,
  review: { state: "none" },
  url: "https://github.com/acme/app/pull/482",
};
const fromFork: PullRequestRow = {
  ...pr,
  key: { type: "pullRequest", remote: "github.com/acme/app", number: 479 },
  title: "Fix CSV export",
  fork: "jdoe/ledger-sync",
  ci: undefined,
  lines: undefined,
};
const forked = pullRequestRow(fromFork);
const all = queueRows(rows.slice(0, 2), [pr, fromFork]);

void test("pull requests follow the local rows, and a tab counts them", () => {
  assert.deepEqual(
    all.map((r) => r.key.type),
    ["workingTree", "branch", "pullRequest", "pullRequest"],
  );
  assert.deepEqual(tabCounts(all), { all: 4, blocking: 0, local: 2, prs: 2 });
  assert.deepEqual(
    filterRows(all, "prs", "").map((r) => r.title),
    ["Add idempotency keys to sync jobs", "Fix CSV export"],
  );
  assert.deepEqual(
    filterRows(all, "local", "").map((r) => r.key.type),
    ["workingTree", "branch"],
  );
});

void test("a pull request is found by its number, its title and where it comes from", () => {
  const titles = (query: string) => filterRows(all, "all", query).map((r) => r.title);
  assert.deepEqual(titles("#482"), ["Add idempotency keys to sync jobs"]);
  assert.deepEqual(titles("482"), ["Add idempotency keys to sync jobs"]);
  assert.deepEqual(titles("csv"), ["Fix CSV export"]);
  assert.deepEqual(titles("jdoe"), ["Fix CSV export"]);
});

void test("a pull request row has its number apart from its title, the repository or fork it comes from, and when it changed", () => {
  const row = pullRequestRow(pr);
  assert.deepEqual(labelOf(row), { number: "#482", text: "Add idempotency keys to sync jobs" });
  assert.deepEqual(labelOf(retry), { text: "feat/retry-policy" });
  assert.equal(sourceText(row), "origin");
  assert.equal(sourceText(forked), "jdoe/ledger-sync");
  assert.equal(updatedText(row, NOW), "12:30 PM");
});

void test("a pull request counts the platform's comments, a local changeset its open threads", () => {
  assert.equal(commentCount(pullRequestRow(pr)), 2);
  assert.equal(commentCount(retry), 2);
  assert.equal(commentCount(tree), 0);
});

void test("the changeset on view shows its own lines; any other pull request what the platform lists", () => {
  const own = { added: 230, removed: 40 };
  assert.deepEqual(shownLines(pullRequestRow(pr), own), own);
  assert.deepEqual(shownLines(pullRequestRow(pr), undefined), { added: 236, removed: 41 });
  assert.equal(shownLines(forked, undefined), undefined);
  assert.equal(shownLines(retry, undefined), undefined);
});

void test("the header names a pull request by number and title, and by number alone while the title is unknown", () => {
  const target = (changeset: Target["changeset"], name: string, title?: string): Target => ({
    changeset,
    name,
    title,
    draft: false,
  });
  const key = { type: "pullRequest", remote: "github.com/acme/app", number: 482 } as const;
  assert.deepEqual(targetLabel(target(key, "#482", "Add idempotency keys")), {
    number: "#482",
    text: "Add idempotency keys",
  });
  assert.deepEqual(targetLabel(target(key, "#482")), { text: "#482" });
  assert.deepEqual(targetLabel(target({ type: "branch", name: "feat/x" }, "feat/x")), { text: "feat/x" });
});

void test("a pull request is asked for by its number, with or without the hash", () => {
  assert.equal(pullRequestNumber("482"), 482);
  assert.equal(pullRequestNumber(" #482 "), 482);
  for (const typed of ["", "#", "0", "-3", "4x", "4 8", "99999999999"])
    assert.equal(pullRequestNumber(typed), undefined, typed);
});
