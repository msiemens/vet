import assert from "node:assert/strict";
import { test } from "node:test";
import type { CheckRun } from "../../generated/CheckRun.ts";
import type { CommitChecks } from "../../generated/CommitChecks.ts";
import { badgeCount, failedFirst, passed } from "./checkRows.ts";

const run = (state: CheckRun["state"]): CheckRun => ({ id: "1", name: "build", state, phase: "finished", log: false });
const commit = (...states: CheckRun["state"][]): CommitChecks => ({
  commit: "abc",
  runs: states.map(run),
  pipelines: [],
  // The combined state is the backend's; these cases only need the states that can come out of it.
  state: states.includes("pending")
    ? "pending"
    : states.includes("failure")
      ? "failure"
      : states.includes("needsApproval")
        ? "needsApproval"
        : states.includes("success")
          ? "success"
          : "neutral",
});

void test("a run that was skipped or decided nothing does not count against the commit", () => {
  assert.deepEqual(passed(commit("success", "skipped", "neutral", "failure", "pending")), { passed: 3, total: 5 });
});

void test("the badge counts what the combined state is about", () => {
  assert.equal(badgeCount(commit("success", "failure", "failure")), 2);
  assert.equal(badgeCount(commit("success", "pending", "failure")), 1);
  assert.equal(badgeCount(commit("success", "success", "skipped")), 3);
  assert.equal(badgeCount(commit("success", "needsApproval", "needsApproval")), 2);
});

void test("failed runs come first and every group keeps its order", () => {
  const named = (name: string, state: CheckRun["state"]): CheckRun => ({ ...run(state), id: name, name });
  const runs = [named("a", "success"), named("b", "failure"), named("c", "pending"), named("d", "failure")];
  assert.deepEqual(
    failedFirst(runs).map((r) => r.id),
    ["b", "d", "a", "c"],
  );
});
