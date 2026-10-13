import assert from "node:assert/strict";
import { test } from "node:test";
import { ciIcon, phaseLook } from "./ciStatus.ts";

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
