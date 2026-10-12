import assert from "node:assert/strict";
import { test } from "node:test";
import { swapTab, tabAfter } from "./tabOrder.ts";

void test("a file's view takes its tab's place and never adds a tab", () => {
  assert.deepEqual(swapTab(["changes", "file:a", "file:b"], "file:a", "text:a"), ["changes", "text:a", "file:b"]);
  assert.deepEqual(swapTab(["changes", "file:a", "text:a"], "file:a", "text:a"), ["changes", "text:a"]);
});

void test("stepping through the tabs wraps around both ends", () => {
  const open = ["changes", "a", "b"];
  assert.equal(tabAfter(open, "changes", 1), "a");
  assert.equal(tabAfter(open, "b", 1), "changes");
  assert.equal(tabAfter(open, "changes", -1), "b");
  assert.equal(tabAfter(open, "a", -1), "changes");
  assert.equal(tabAfter(["changes"], "changes", 1), "changes");
});
