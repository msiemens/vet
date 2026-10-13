import assert from "node:assert/strict";
import { test } from "node:test";
import { deltaParts, deltaText } from "./lineDelta.ts";

void test("added and removed lines are signed, the removed with a true minus", () => {
  assert.deepEqual(deltaParts(12, 3), ["+12", "−3"]);
  assert.deepEqual(deltaParts(0, 0), ["+0", "−0"]);
});

void test("the counts as text are separated by a space", () => {
  assert.equal(deltaText(12, 3), "+12 −3");
});
