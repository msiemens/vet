import assert from "node:assert/strict";
import { test } from "node:test";
import type { Completion } from "../../../generated/Completion.ts";
import { countText } from "./fileHeader.ts";

const completion = (c: Partial<Completion>) =>
  ({ hunks: 0, seenHunks: 0, items: 0, acknowledgedItems: 0, ...c }) as Completion;

void test("a file with hunks reports them", () => {
  assert.equal(countText(completion({ hunks: 4, seenHunks: 1 })), "1/4 hunks seen");
});

void test("a file with only non-text items reports just those", () => {
  assert.equal(countText(completion({ items: 2, acknowledgedItems: 1 })), "1/2 other");
});

void test("a file with both reports both", () => {
  assert.equal(
    countText(completion({ hunks: 3, seenHunks: 3, items: 1, acknowledgedItems: 0 })),
    "3/3 hunks seen · 0/1 other",
  );
});

void test("a file with nothing to review still says so", () => {
  assert.equal(countText(completion({})), "0/0 hunks seen");
});
