import assert from "node:assert/strict";
import { test } from "node:test";
import { flaggedWarning } from "./finishWarning.ts";

void test("finishing warns with the number of hunks still flagged, and says nothing without any", () => {
  assert.equal(flaggedWarning(0), undefined);
  assert.equal(flaggedWarning(1), "1 hunk is still flagged for later review.");
  assert.equal(flaggedWarning(3), "3 hunks are still flagged for later review.");
});
