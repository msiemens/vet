import assert from "node:assert/strict";
import { test } from "node:test";
import type { ThreadTarget } from "../../../generated/ThreadTarget.ts";
import { draftTitle, newCommentKey } from "./threadCard.ts";

void test("a draft's title says what the comment is about", () => {
  assert.equal(draftTitle({ type: "range", side: "target", path: "a.rs", start: 4, len: 2 }), "Comment on L5–6");
  assert.equal(draftTitle({ type: "range", side: "base", path: "a.rs", start: 4, len: 1 }), "Comment on base L5");
  assert.equal(draftTitle({ type: "file", side: "target", path: "a.rs" }), "Comment on this file");
});

void test("an unsent new comment is kept per project and changeset, and per place in it", () => {
  const here: ThreadTarget = { type: "range", side: "target", path: "a.rs", start: 4, len: 2 };
  const key = newCommentKey("1/branch:main", here);
  assert.equal(key, newCommentKey("1/branch:main", { ...here }));
  assert.notEqual(key, newCommentKey("2/branch:main", here), "another project");
  assert.notEqual(key, newCommentKey("1/pullRequest:o/r#3", here), "another changeset");
  assert.notEqual(key, newCommentKey("1/branch:main", { ...here, start: 5 }), "another line");
});
