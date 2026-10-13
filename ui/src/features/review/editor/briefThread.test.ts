import assert from "node:assert/strict";
import { test } from "node:test";
import { briefMessages, isUnread, needsShowAll } from "./briefThread.ts";

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

void test("only what others wrote after the viewer last looked is new", () => {
  assert.equal(isUnread({ mine: false, at: 11 }, 10), true);
  assert.equal(isUnread({ mine: false, at: 10 }, 10), false);
  assert.equal(isUnread({ mine: true, at: 11 }, 10), false);
  assert.equal(isUnread({ mine: false, at: 11 }, undefined), false, "nothing is new before the first look");
});
