import assert from "node:assert/strict";
import { test } from "node:test";
import type { Thread } from "../../../generated/Thread.ts";
import type { ThreadMessage } from "../../../generated/ThreadMessage.ts";
import type { ThreadTarget } from "../../../generated/ThreadTarget.ts";
import { draftTitle, foldedSummary, newCommentKey } from "./threadCard.ts";

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

const message = (id: number, author: string, body: string): ThreadMessage => ({
  id,
  author,
  body,
  at: 0,
  mine: false,
  avatar: null,
  state: { type: "posted" },
  editedAt: null,
  conflict: null,
});

const resolved = (messages: ThreadMessage[], resolvedBy: string | null): Thread => ({
  id: 1,
  kind: "range",
  side: "target",
  path: "a.rs",
  original: null,
  location: null,
  targetLine: 1,
  anchor: { type: "exact", whitespaceChanged: false },
  messages,
  status: "resolved",
  platform: { posted: true, resolvable: true, resolvedBy },
});

void test("a folded thread says who started it and what they said, without markup", () => {
  const summary = foldedSummary(resolved([message(1, "Marco", "**Log** the attempt\n\nsecond paragraph")], "Lena"));
  assert.equal(summary.author, "Marco");
  assert.equal(summary.text, "Log the attempt");
  assert.equal(summary.resolved, "Resolved by Lena");
});

void test("a folded thread counts its comments and names each author once, only when there are replies", () => {
  const alone = foldedSummary(resolved([message(1, "Marco", "a")], null));
  assert.equal(alone.count, null);
  assert.equal(alone.resolved, "Resolved");
  const talk = foldedSummary(
    resolved([message(1, "Marco", "a"), message(2, "Lena", "b"), message(3, "Marco", "c")], null),
  );
  assert.equal(talk.count, "3 comments");
  assert.deepEqual(
    talk.participants.map((p) => p.author),
    ["Marco", "Lena"],
  );
});
