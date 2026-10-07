import assert from "node:assert/strict";
import { test } from "node:test";
import { percentages } from "./completion.ts";

void test("lines and other items count alike; an old base is a share of the same total", () => {
  assert.deepEqual(percentages({ lines: 6, seenLines: 3, oldBaseLines: 1, items: 2, acknowledgedItems: 1 }), {
    seenPct: 50,
    oldPct: 12.5,
  });
});

void test("nothing to review is zero percent", () => {
  assert.deepEqual(percentages({ lines: 0, seenLines: 0, oldBaseLines: 0, items: 0, acknowledgedItems: 0 }), {
    seenPct: 0,
    oldPct: 0,
  });
});
