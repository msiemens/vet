import assert from "node:assert/strict";
import { test } from "node:test";
import type { Thread } from "../../generated/Thread.ts";
import type { ThreadMessage } from "../../generated/ThreadMessage.ts";
import { briefMessages, foldedSummary, needsShowAll } from "./commentCard.ts";

void test("a long thread shows its first and last message and counts those between", () => {
  assert.deepEqual(briefMessages([1, 2]), { shown: [1, 2], between: 0 });
  assert.deepEqual(briefMessages([1, 2, 3]), { shown: [1, 3], between: 1 });
  assert.deepEqual(briefMessages([1, 2, 3, 4, 5]), { shown: [1, 5], between: 3 });
});

void test("a message offers Show all when four lines would cut it", () => {
  assert.equal(needsShowAll("short"), false);
  assert.equal(needsShowAll("a\nb\nc\nd"), false);
  assert.equal(needsShowAll("a\nb\nc\nd\ne"), true);
  assert.equal(needsShowAll("x".repeat(300)), true);
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
