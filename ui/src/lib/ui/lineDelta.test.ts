import assert from "node:assert/strict";
import { test } from "node:test";
import { deltaParts, deltaText, sumDelta } from "./lineDelta.ts";

void test("added and removed lines are signed, the removed with a true minus", () => {
  assert.deepEqual(deltaParts(12, 3), ["+12", "−3"]);
  assert.deepEqual(deltaParts(0, 0), ["+0", "−0"]);
});

void test("the counts as text are separated by a space", () => {
  assert.equal(deltaText(12, 3), "+12 −3");
});

void test("changes add up per side, and nothing adds up to nothing", () => {
  assert.deepEqual(
    sumDelta([
      { added: 5, removed: 1 },
      { added: 2, removed: 7 },
    ]),
    { added: 7, removed: 8 },
  );
  assert.deepEqual(sumDelta([]), { added: 0, removed: 0 });
});
