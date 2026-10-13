import assert from "node:assert/strict";
import { test } from "node:test";
import { ciIcon } from "./ciStatus.ts";

void test("only checks that passed, failed, run or wait for approval have an icon", () => {
  assert.deepEqual(
    (["success", "failure", "pending", "needsApproval", "neutral", "skipped", undefined] as const).map(
      (s) => ciIcon(s)?.tone,
    ),
    ["ok", "failing", "running", "waiting", undefined, undefined, undefined],
  );
});
