import assert from "node:assert/strict";
import { test } from "node:test";
import { lensOnSelect, settleCommits, toolFor } from "./commitsTool.ts";

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

void test("settleCommits: picking the Commits tool asks for the by-commit lens", () => {
  const settled = settleCommits(
    { tool: "changes", lens: "squash", commits: 3 },
    { tool: "commits", lens: "squash", commits: 3 },
  );
  assert.deepEqual(settled, { tool: "commits", lens: "squash", commits: 3, askLens: "perCommit" });
});

void test("settleCommits: leaving the Commits tool asks for all changes", () => {
  const settled = settleCommits(
    { tool: "commits", lens: "perCommit", commits: 3 },
    { tool: "all", lens: "perCommit", commits: 3 },
  );
  assert.deepEqual(settled, { tool: "all", lens: "perCommit", commits: 3, askLens: "squash" });
});

void test("settleCommits: moving between other tools leaves the lens as it is", () => {
  const settled = settleCommits(
    { tool: "changes", lens: "squash", commits: 3 },
    { tool: "all", lens: "squash", commits: 3 },
  );
  assert.deepEqual(settled, { tool: "all", lens: "squash", commits: 3, askLens: undefined });
});

void test("settleCommits: the Commits tool gives way when there is nothing to review by commit", () => {
  const settled = settleCommits(
    { tool: "changes", lens: "squash", commits: 1 },
    { tool: "commits", lens: "squash", commits: 1 },
  );
  assert.deepEqual(settled, { tool: "changes", lens: "squash", commits: 1 });
});

void test("settleCommits: a lens set elsewhere selects the tool that goes with it", () => {
  assert.deepEqual(
    settleCommits({ tool: "changes", lens: "squash", commits: 3 }, { tool: "changes", lens: "perCommit", commits: 3 }),
    { tool: "commits", lens: "perCommit", commits: 3 },
  );
  assert.deepEqual(
    settleCommits({ tool: "commits", lens: "perCommit", commits: 3 }, { tool: "commits", lens: "squash", commits: 3 }),
    { tool: "changes", lens: "squash", commits: 3 },
  );
});

void test("settleCommits: a lens with nothing to review by commit leaves the tool", () => {
  assert.deepEqual(
    settleCommits({ tool: "changes", lens: "squash", commits: 1 }, { tool: "changes", lens: "perCommit", commits: 1 }),
    { tool: "changes", lens: "perCommit", commits: 1 },
  );
});

void test("settleCommits: losing commits leaves the by-commit tool", () => {
  const settled = settleCommits(
    { tool: "commits", lens: "perCommit", commits: 3 },
    { tool: "commits", lens: "perCommit", commits: 1 },
  );
  assert.deepEqual(settled, { tool: "changes", lens: "perCommit", commits: 1 });
  // The tool then moved, so the next step asks for all changes.
  assert.equal(settleCommits({ tool: "commits", lens: "perCommit", commits: 1 }, settled).askLens, "squash");
});

void test("settleCommits: a pair that already agrees asks for nothing", () => {
  assert.deepEqual(
    settleCommits(
      { tool: "commits", lens: "perCommit", commits: 3 },
      { tool: "commits", lens: "perCommit", commits: 3 },
    ),
    { tool: "commits", lens: "perCommit", commits: 3 },
  );
  assert.deepEqual(
    settleCommits({ tool: "changes", lens: "squash", commits: 3 }, { tool: "changes", lens: "squash", commits: 3 }),
    { tool: "changes", lens: "squash", commits: 3 },
  );
});
