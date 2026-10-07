import assert from "node:assert/strict";
import { test } from "node:test";
import { parseLines } from "./parseLines.ts";

void test("lines are trimmed and blank ones dropped", () => {
  assert.deepEqual(parseLines("  src/a \n\n\t\n*.lock\r\n"), ["src/a", "*.lock"]);
});

void test("an empty field has no entries", () => {
  assert.deepEqual(parseLines(""), []);
});
