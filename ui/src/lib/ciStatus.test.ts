import assert from "node:assert/strict";
import { test } from "node:test";
import { ciIcon } from "./ciStatus.ts";

void test("only checks that passed, failed or run have an icon", () => {
  assert.deepEqual(
    (["success", "failure", "pending", "neutral", "skipped", undefined] as const).map((s) => ciIcon(s)?.tone),
    ["ok", "failing", "running", undefined, undefined, undefined],
  );
});
