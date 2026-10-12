import assert from "node:assert/strict";
import { test } from "node:test";
import { tabAfter } from "./tabOrder.ts";

void test("stepping through the tabs wraps around both ends", () => {
  const open = ["changes", "a", "b"];
  assert.equal(tabAfter(open, "changes", 1), "a");
  assert.equal(tabAfter(open, "b", 1), "changes");
  assert.equal(tabAfter(open, "changes", -1), "b");
  assert.equal(tabAfter(open, "a", -1), "changes");
  assert.equal(tabAfter(["changes"], "changes", 1), "changes");
});
