import assert from "node:assert/strict";
import { test } from "node:test";
import { draftTitle } from "./threadCard.ts";

void test("a draft's title says what the comment is about", () => {
  assert.equal(draftTitle({ type: "range", side: "target", path: "a.rs", start: 4, len: 2 }), "Comment on L5–6");
  assert.equal(draftTitle({ type: "range", side: "base", path: "a.rs", start: 4, len: 1 }), "Comment on base L5");
  assert.equal(draftTitle({ type: "file", side: "target", path: "a.rs" }), "Comment on this file");
});
