import assert from "node:assert/strict";
import { test } from "node:test";
import type { CommitInfo } from "../../generated/CommitInfo.ts";
import { searchCommits } from "./commitSearch.ts";

const commit = (id: string, subject: string, author: string): CommitInfo => ({
  id,
  shortId: id.slice(0, 7),
  subject,
  author,
  time: 0,
});
const commits = [
  commit("a41c9e2f0000", "net: tweak retry", "Ada Reviewer"),
  commit("0f3b7d1aa000", "docs: fix typo", "Grace Hopper"),
];

void test("every term must match the id, subject or author", () => {
  assert.equal(searchCommits(commits, "").length, 2);
  assert.deepEqual(
    searchCommits(commits, "ada retry").map((c) => c.id),
    ["a41c9e2f0000"],
  );
  assert.deepEqual(
    searchCommits(commits, "0F3B").map((c) => c.id),
    ["0f3b7d1aa000"],
  );
  assert.equal(searchCommits(commits, "retry typo").length, 0);
});
