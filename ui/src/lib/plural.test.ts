import assert from "node:assert/strict";
import { test } from "node:test";
import { noun, plural } from "./plural.ts";

void test("a count takes the singular for one and the plural for any other number", () => {
  assert.equal(plural(1, "file"), "1 file");
  assert.equal(plural(0, "file"), "0 files");
  assert.equal(plural(2, "reply", "replies"), "2 replies");
  assert.equal(noun(1, "reply", "replies"), "reply");
});
