import assert from "node:assert/strict";
import { test } from "node:test";
import { letterOfChange, letterOfStatus } from "./changeLetter.ts";

void test("a symbol is marked as a file that changed the same way", () => {
  assert.equal(letterOfChange("modified"), letterOfStatus("modified"));
  assert.equal(letterOfChange("added"), letterOfStatus("added"));
  assert.equal(letterOfChange("removed"), letterOfStatus("deleted"));
});

void test("a renamed file has a letter of its own", () => {
  const others = [letterOfStatus("modified"), letterOfStatus("added"), letterOfStatus("deleted")];
  assert.ok(!others.includes(letterOfStatus("renamed")));
});
