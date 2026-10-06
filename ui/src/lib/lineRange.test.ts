import assert from "node:assert/strict";
import { test } from "node:test";
import { lineRange } from "./lineRange.ts";

void test("lines are shown 1-based, a range with its last line", () => {
  assert.equal(lineRange(11, 1), "L12");
  assert.equal(lineRange(11, 3), "L12–14");
  assert.equal(lineRange(0, 0), "L1");
});
