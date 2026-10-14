import assert from "node:assert/strict";
import { test } from "node:test";
import { dockLimits } from "./dock.ts";

void test("a review area too small for both gives the dock its smallest height only", () => {
  const [min, max] = dockLimits(100);
  assert.equal(max, min);
});
