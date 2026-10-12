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

void test("a commit from earlier this week says the weekday and the time of day", () => {
  const now = new Date(2026, 9, 7, 15, 30).getTime();
  const base = unit({ type: "commit" });
  const commit = { ...base.commit, time: new Date(2026, 9, 5, 14, 2).getTime() / 1000 };
  assert.equal(commitRow({ ...base, commit }, now).when, "Mon 2:02 PM");
});

void test("a clean merge is muted", () => {
  const row = commitRow(
    unit({ type: "merge", clean: true, remerged: true }, { lines: 0, seenLines: 0, seenPercent: 0 }),
  );
  assert.deepEqual([row.merge, row.clean, row.notRemerged], [true, true, false]);
});

void test("a merge compared with its first parent says so", () => {
  assert.equal(commitRow(unit({ type: "merge", clean: false, remerged: false })).notRemerged, true);
  assert.equal(commitRow(unit({ type: "merge", clean: false, remerged: true })).notRemerged, false);
});
