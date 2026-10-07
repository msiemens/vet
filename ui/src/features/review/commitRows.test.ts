import assert from "node:assert/strict";
import { test } from "node:test";
import type { CommitKind } from "../../generated/CommitKind.ts";
import type { CommitUnit } from "../../generated/CommitUnit.ts";
import { commitRow } from "./commitRows.ts";

function unit(kind: CommitKind, completion: Partial<CommitUnit["completion"]> = {}): CommitUnit {
  return {
    commit: { id: "a7d93e2ffff", shortId: "a7d93e2", subject: "Add job_keys table", author: "Lena", time: 0 },
    body: "",
    parents: ["p1"],
    kind,
    delta: { added: 62, removed: 0 },
    completion: {
      lines: 62,
      seenLines: 62,
      oldBaseLines: 0,
      hunks: 1,
      seenHunks: 1,
      items: 0,
      acknowledgedItems: 0,
      complete: true,
      seenPercent: 100,
      oldBasePercent: 0,
      ...completion,
    },
  };
}

void test("a fully seen commit is done and carries its own numbers", () => {
  const row = commitRow(unit({ type: "commit" }));
  assert.deepEqual(
    [row.done, row.merge, row.clean, row.added, row.removed, row.seen],
    [true, false, false, 62, 0, 100],
  );
});

void test("a commit with part of its diff unseen is not done", () => {
  const row = commitRow(unit({ type: "commit" }, { complete: false, seenPercent: 40, oldBasePercent: 10 }));
  assert.deepEqual([row.done, row.seen, row.oldBase], [false, 40, 10]);
});

void test("a clean merge is muted and counts as nothing to do rather than as done", () => {
  const row = commitRow(
    unit({ type: "merge", clean: true, remerged: true }, { lines: 0, seenLines: 0, seenPercent: 0 }),
  );
  assert.deepEqual([row.merge, row.clean, row.done, row.notRemerged], [true, true, false, false]);
});

void test("a merge compared with its first parent says so", () => {
  assert.equal(commitRow(unit({ type: "merge", clean: false, remerged: false })).notRemerged, true);
  assert.equal(commitRow(unit({ type: "merge", clean: false, remerged: true })).notRemerged, false);
});
