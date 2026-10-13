import assert from "node:assert/strict";
import { test } from "node:test";
import { DOCK_MIN, dockLimits } from "./dock.ts";

void test("the dock leaves the review above it some room", () => {
  const [min, max] = dockLimits(800);
  assert.equal(min, DOCK_MIN);
  assert.ok(max > min && max < 800);
});

void test("a review area too small for both gives the dock its smallest height only", () => {
  const [min, max] = dockLimits(100);
  assert.equal(max, min);
});
