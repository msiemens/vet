import assert from "node:assert/strict";
import { test } from "node:test";
import type { CommitChecks } from "../generated/CommitChecks";
import { ciIcon, headerTitle, phaseLook } from "./ciStatus.ts";

void test("a job reads by how far it has come before how it ended", () => {
  assert.equal(phaseLook("pending", "queued").title, "Queued");
  assert.equal(phaseLook("pending", "running").title, "Running");
  assert.equal(phaseLook("neutral", "cancelled").title, "Cancelled");
  assert.equal(phaseLook("success", "finished").title, "Passed");
  assert.equal(phaseLook("failure", "finished").title, "Failed");
  assert.equal(phaseLook("needsApproval", "finished").title, "Needs approval");
  assert.equal(phaseLook("skipped", "finished").title, "Skipped");
});

void test("only checks that passed, failed, run or wait for approval have an icon", () => {
  assert.deepEqual(
    (["success", "failure", "pending", "needsApproval", "neutral", "skipped", undefined] as const).map(
      (s) => ciIcon(s)?.tone,
    ),
    ["ok", "failing", "running", "waiting", undefined, undefined, undefined],
  );
});

void test("the checks button names what failed, and says the result is old while the platform limits requests", () => {
  const checks = {
    commit: "a".repeat(40),
    state: "failure",
    runs: [
      { state: "failure", name: "job 1" },
      { state: "failure", name: "job 2" },
    ],
    pipelines: [],
  } as unknown as CommitChecks;
  assert.equal(headerTitle(checks), "Checks failing: job 1, job 2");
  assert.equal(
    headerTitle({ ...checks, state: "pending" }, { until: "14:32", asOf: "14:21", platform: "GitHub" }),
    "Checks running. GitHub is limiting requests until 14:32; as of 14:21.",
  );
});
