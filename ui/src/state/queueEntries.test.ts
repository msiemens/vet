import assert from "node:assert/strict";
import { test } from "node:test";
import type { ChangesetRow } from "../generated/ChangesetRow";
import type { Completion } from "../generated/Completion";
import type { LineDelta } from "../generated/LineDelta";
import type { PullRequestRow } from "../generated/PullRequestRow";
import { keyId } from "./changesetKey.ts";
import { queueEntries, queueRows } from "./queueEntries.ts";

const completion = (seenPercent: number, oldBasePercent: number): Completion => ({
  lines: 0,
  seenLines: 0,
  oldBaseLines: 0,
  hunks: 0,
  seenHunks: 0,
  flaggedHunks: 0,
  items: 0,
  acknowledgedItems: 0,
  complete: false,
  seenPercent,
  oldBasePercent,
});

const tree: ChangesetRow = {
  key: { type: "workingTree" },
  title: "Working tree",
  checkedOut: false,
  branch: "main",
  openThreads: 0,
  review: { state: "none" },
};
const branch: ChangesetRow = {
  key: { type: "branch", name: "feat/x" },
  title: "feat/x",
  checkedOut: false,
  openThreads: 0,
  divergence: { ahead: 2, behind: 0 },
  lastCommit: 1,
  review: { state: "open", version: 4, startedAt: 0, progress: { seenPercent: 12, oldBasePercent: 3 } },
};
const pr: PullRequestRow = {
  key: { type: "pullRequest", remote: "github.com/acme/app", number: 482 },
  title: "Add idempotency keys",
  author: "lena",
  authorName: "Lena Berg",
  sourceBranch: "lena/keys",
  targetBranch: "main",
  lines: { added: 236, removed: 41 },
  comments: 0,
  draft: false,
  changesRequested: [],
  waitingForMe: false,
  unresolvedThreads: 0,
  updatedAt: 1,
  review: { state: "none" },
  url: "https://github.com/acme/app/pull/482",
};
const rows = queueRows([tree, branch], [pr]);
const overlay = { layout: { delta: { added: 5, removed: 2 } }, facts: { completion: completion(40, 10) } };

void test("the entries are the local changesets then the pull requests, only the selected one on view", () => {
  const entries = queueEntries(rows, keyId(branch.key), overlay, new Map());
  assert.deepEqual(
    entries.map((e) => e.key.type),
    ["workingTree", "branch", "pullRequest"],
  );
  assert.deepEqual(
    entries.map((e) => e.onView),
    [false, true, false],
  );
});

void test("the selected row shows the overlay's delta and completion, not what was counted for it", () => {
  const counted = new Map([[keyId(branch.key), { added: 90, removed: 30 }]]);
  const entries = queueEntries(rows, keyId(branch.key), overlay, counted);
  assert.deepEqual(entries[1].changes, { kind: "lines", lines: { added: 5, removed: 2 } });
  assert.deepEqual(entries[1].reviewed, { kind: "progress", seen: 40, oldBase: 10 });
});

void test("the rows that are not on view keep their counted lines, stored review and platform numbers", () => {
  const counted = new Map<string, LineDelta | null>([[keyId(branch.key), null]]);
  const entries = queueEntries(rows, undefined, overlay, counted);
  assert.deepEqual(
    entries.map((e) => e.onView),
    [false, false, false],
  );
  // The working tree has no count to wait for; a counted `null` says there is none to count.
  assert.deepEqual(entries[0].changes, { kind: "none" });
  assert.deepEqual(entries[1].changes, { kind: "none" });
  assert.deepEqual(entries[1].reviewed, { kind: "progress", seen: 12, oldBase: 3 });
  // The platform's own numbers fill in until the backend counts the pull request.
  assert.deepEqual(entries[2].changes, { kind: "lines", lines: { added: 236, removed: 41 } });
});
