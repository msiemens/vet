import assert from "node:assert/strict";
import { test } from "node:test";
import { lensOnSelect, toolFor } from "./commitsTool.ts";

void test("picking Commits reviews by commit and leaving it returns to all changes", () => {
  assert.equal(lensOnSelect("changes", "commits"), "perCommit");
  assert.equal(lensOnSelect("comments", "commits"), "perCommit");
  assert.equal(lensOnSelect("commits", "all"), "squash");
  assert.equal(lensOnSelect("changes", "all"), undefined);
  assert.equal(lensOnSelect("commits", "commits"), undefined);
});

void test("a changeset reopened by commit selects the Commits tool", () => {
  assert.equal(toolFor("changes", "perCommit", 3), "commits");
  assert.equal(toolFor("all", "perCommit", 3), "commits");
  assert.equal(toolFor("changes", "squash", 3), "changes");
});

void test("the Commits tool gives way when there is nothing to review by commit", () => {
  assert.equal(toolFor("commits", "perCommit", 1), "changes");
  assert.equal(toolFor("commits", "perCommit", 0), "changes");
  assert.equal(toolFor("commits", "squash", 3), "changes");
  assert.equal(toolFor("commits", "perCommit", 3), "commits");
  assert.equal(toolFor("commits", undefined, 3), "commits");
});
