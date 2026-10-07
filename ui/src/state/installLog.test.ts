import assert from "node:assert/strict";
import { test } from "node:test";
import { appendLog } from "./installLog.ts";

void test("the install log keeps the newest lines", () => {
  let lines: string[] = [];
  for (let i = 0; i < 5; i++) lines = appendLog(lines, `l${i}`, 3);
  assert.deepEqual(lines, ["l2", "l3", "l4"]);
});
