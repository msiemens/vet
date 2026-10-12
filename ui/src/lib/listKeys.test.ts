import assert from "node:assert/strict";
import { test } from "node:test";
import { moveHighlight } from "./listKeys.ts";

void test("the highlight starts at the end the key comes from and stops at both ends", () => {
  assert.equal(moveHighlight(-1, "down", 3), 0);
  assert.equal(moveHighlight(-1, "up", 3), 2);
  assert.equal(moveHighlight(2, "down", 3), 2);
  assert.equal(moveHighlight(0, "up", 3), 0);
  assert.equal(moveHighlight(1, "down", 3), 2);
});

void test("an empty list has no highlight", () => {
  assert.equal(moveHighlight(-1, "down", 0), -1);
  assert.equal(moveHighlight(4, "up", 0), -1);
});

void test("the highlight of a list that shrank comes back into it", () => {
  assert.equal(moveHighlight(9, "up", 3), 2);
  assert.equal(moveHighlight(9, "down", 3), 2);
});
