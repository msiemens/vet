import assert from "node:assert/strict";
import { test } from "node:test";
import { COMMITS, lensOnSelect, toolFor } from "./commitsTool.ts";

void test("picking Commits reviews by commit and leaving it returns to all changes", () => {
  assert.equal(lensOnSelect("changes", "commits"), "perCommit");
  assert.equal(lensOnSelect("comments", "commits"), "perCommit");
  assert.equal(lensOnSelect("commits", "all"), "squash");
  assert.equal(lensOnSelect("commits", "changes"), "squash");
  assert.equal(lensOnSelect("changes", "all"), undefined);
  assert.equal(lensOnSelect("commits", "commits"), undefined);
});

void test("a changeset reviewed by commit shows the Commits tool", () => {
  assert.equal(toolFor("changes", "perCommit", 3), COMMITS);
  assert.equal(toolFor("all", "perCommit", 3), COMMITS);
  assert.equal(toolFor("comments", "perCommit", 3), COMMITS);
});

void test("otherwise the rail shows the tool chosen last", () => {
  assert.equal(toolFor("changes", "squash", 3), "changes");
  assert.equal(toolFor("all", "squash", 3), "all");
  assert.equal(toolFor("comments", undefined, 3), "comments");
  assert.equal(toolFor("all", "squash", 1), "all");
});

void test("a lens with nothing to review by commit leaves the chosen tool", () => {
  assert.equal(toolFor("changes", "perCommit", 1), "changes");
  assert.equal(toolFor("all", "perCommit", 0), "all");
});
