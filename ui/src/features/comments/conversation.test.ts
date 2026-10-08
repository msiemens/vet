import assert from "node:assert/strict";
import { test } from "node:test";
import type { ConversationEntry } from "../../generated/ConversationEntry.ts";
import type { Thread } from "../../generated/Thread.ts";
import type { ThreadMessage } from "../../generated/ThreadMessage.ts";
import { changesetThreads, draftComments, isOwnComment, reviewThreads, verdictChip } from "./conversation.ts";

const message = (id: number, over: Partial<ThreadMessage> = {}): ThreadMessage => ({
  id,
  author: "ada",
  body: `message ${id}`,
  at: id,
  mine: true,
  avatar: null,
  state: { type: "draft" },
  editedAt: null,
  conflict: null,
  ...over,
});
const line = (id: number, over: Partial<Thread> = {}): Thread => ({
  id,
  kind: "range",
  side: "target",
  path: "a.rs",
  original: { path: "a.rs", start: 4, len: 1 },
  location: { path: "a.rs", start: 4, len: 1 },
  anchor: { type: "exact", whitespaceChanged: false },
  messages: [message(id)],
  status: "open",
  platform: { posted: false, resolvable: true, resolvedBy: null },
  ...over,
});
const whole = (id: number, resolvable: boolean, messages = [message(id)]): Thread =>
  line(id, {
    kind: "changeset",
    path: null,
    original: null,
    location: null,
    anchor: null,
    messages,
    platform: { posted: false, resolvable, resolvedBy: null },
  });
const entry = (kind: ConversationEntry["kind"]): ConversationEntry => ({
  id: "IC1",
  author: "ada",
  avatar: null,
  at: 0,
  kind,
  body: "",
});
const ids = (list: Thread[]) => list.map((t) => t.id);

void test("the Review tab lists the threads on files and lines, and those on the whole changeset are apart", () => {
  const file = line(2, { kind: "file", location: null, original: null });
  const all = [line(1), file, whole(3, true)];
  assert.deepEqual(ids(reviewThreads(all)), [1, 2]);
  assert.deepEqual(ids(changesetThreads(all)), [3]);
});

void test("the comments on the pull request that wait for Submit review are the unposted ones of flat threads, oldest first", () => {
  const drafts = draftComments([
    whole(1, false, [message(30)]),
    whole(2, false, [message(10, { state: { type: "posted" } }), message(20)]),
    // A thread on the pull request as a whole where the platform keeps those as threads.
    whole(3, true),
    line(4),
  ]);
  assert.deepEqual(
    drafts.map((d) => [d.thread.id, d.message.id]),
    [
      [2, 20],
      [1, 30],
    ],
  );
});

void test("only the viewer's comments can be changed, never a review", () => {
  assert.equal(isOwnComment(entry({ type: "comment", mine: true })), true);
  assert.equal(isOwnComment(entry({ type: "comment", mine: false })), false);
  assert.equal(isOwnComment(entry({ type: "review", verdict: "approved" })), false);
});

void test("a review's chip names its verdict; a comment and a withdrawn review have none", () => {
  const chip = (kind: ConversationEntry["kind"]) => verdictChip(entry(kind))?.label;
  assert.equal(chip({ type: "review", verdict: "approved" }), "Approved");
  assert.equal(chip({ type: "review", verdict: "changesRequested" }), "Changes requested");
  assert.equal(chip({ type: "review", verdict: "commented" }), "Commented");
  assert.equal(chip({ type: "review", verdict: null }), undefined);
  assert.equal(chip({ type: "comment", mine: true }), undefined);
});
