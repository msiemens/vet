import assert from "node:assert/strict";
import { test } from "node:test";
import { briefMessages, needsShowAll } from "./briefThread.ts";

void test("a long thread shows its first and last message and counts those between", () => {
  assert.deepEqual(briefMessages([1, 2]), { shown: [1, 2], between: 0 });
  assert.deepEqual(briefMessages([1, 2, 3]), { shown: [1, 3], between: 1 });
  assert.deepEqual(briefMessages([1, 2, 3, 4, 5]), { shown: [1, 5], between: 3 });
});

void test("a message offers Show all when four lines would cut it", () => {
  assert.equal(needsShowAll("short"), false);
  assert.equal(needsShowAll("a\nb\nc\nd"), false);
  assert.equal(needsShowAll("a\nb\nc\nd\ne"), true);
  assert.equal(needsShowAll("x".repeat(300)), true);
});
