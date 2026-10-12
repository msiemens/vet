import assert from "node:assert/strict";
import { test } from "node:test";
import type { Thread } from "../../generated/Thread.ts";
import type { ThreadMessage } from "../../generated/ThreadMessage.ts";
import {
  canPost,
  destination,
  outgoingComments,
  outgoingKey,
  outgoingTitle,
  postLabel,
  summarizeOutgoing,
  voteChoices,
  voteText,
} from "./outgoing.ts";

const message = (id: number, state: ThreadMessage["state"] = { type: "draft" }): ThreadMessage => ({
  id,
  author: "ada",
  body: `message ${id}`,
  at: id,
  mine: true,
  avatar: null,
  state,
  editedAt: null,
  conflict: null,
});
const thread = (
  id: number,
  path: string | null,
  start: number,
  messages: ThreadMessage[],
  onPlatform = true,
): Thread => ({
  id,
  kind: path === null ? "changeset" : "range",
  side: path === null ? null : "target",
  path,
  original: path === null ? null : { path, start, len: 1 },
  location: path === null ? null : { path, start, len: 1 },
  targetLine: path === null ? null : start,
  anchor: path === null ? null : { type: "exact", whitespaceChanged: false },
  messages,
  status: "open",
  platform: onPlatform ? { posted: false, resolvable: true, resolvedBy: null } : null,
});
const posted = { type: "posted" } as const;
const failed = { type: "failed", error: "locked" } as const;
const posting = { type: "posting" } as const;

void test("the outgoing comments are the unposted ones of a platform's threads, in the Comments tool window's order", () => {
  const list = outgoingComments([
    thread(1, "b.rs", 30, [message(10, posted), message(11), message(12, failed)]),
    thread(2, "a.rs", 4, [message(20, posting)]),
    thread(3, null, 0, [message(30)]),
    thread(4, "b.rs", 2, [message(40, posted)]),
    // Without a platform nothing is posted, so nothing is outgoing.
    thread(5, "a.rs", 1, [message(50)], false),
  ]);
  assert.deepEqual(
    list.map((o) => o.message.id),
    [30, 20, 11, 12],
  );
});

void test("the button counts what is outgoing, how much the platform refused, and whether some is on its way", () => {
  const list = outgoingComments([thread(1, "a.rs", 1, [message(1), message(2, failed), message(3, failed)])]);
  assert.deepEqual(summarizeOutgoing(list, null), { count: 3, failed: 2, posting: false });
  const sending = outgoingComments([thread(1, "a.rs", 1, [message(1, posting)])]);
  assert.deepEqual(summarizeOutgoing(sending, null), { count: 1, failed: 0, posting: true });
  assert.deepEqual(summarizeOutgoing([], null), { count: 0, failed: 0, posting: false });
});

void test("a staged vote is outgoing like a comment, in its own state", () => {
  const list = outgoingComments([thread(1, "a.rs", 1, [message(1)])]);
  assert.deepEqual(summarizeOutgoing(list, { verdict: "approved", state: { type: "draft" } }), {
    count: 2,
    failed: 0,
    posting: false,
  });
  assert.deepEqual(summarizeOutgoing(list, { verdict: "approved", state: failed }), {
    count: 2,
    failed: 1,
    posting: false,
  });
  assert.deepEqual(summarizeOutgoing([], { verdict: "approved", state: posting }), {
    count: 1,
    failed: 0,
    posting: true,
  });
});

void test("the popover names the staged vote by the platform's label and the button says what is outgoing", () => {
  const traits = { verdicts: [{ id: "approved", label: "Approve" }] };
  assert.equal(voteText({ traits }, { verdict: "approved", state: { type: "draft" } }), "Vote: Approve");
  assert.equal(voteText({ traits }, { verdict: "other", state: { type: "draft" } }), "Vote: other");
  const summary = { count: 3, failed: 1, posting: false };
  assert.equal(outgoingTitle(summary, false), "1 of 3 outgoing comments failed to post");
  assert.equal(outgoingTitle({ ...summary, failed: 0 }, true), "3 outgoing items");
  assert.equal(outgoingTitle({ count: 0, failed: 0, posting: false }, false), "No outgoing comments");
});

void test("the vote picker lists the platform's verdicts and marks the staged vote, else the one the viewer cast, else none", () => {
  const traits = {
    verdicts: [
      { id: "approved", label: "Approve" },
      { id: "waiting", label: "Wait for Author" },
      { id: "rejected", label: "Reject" },
    ],
  };
  const marked = (info: Parameters<typeof voteChoices>[0]) => voteChoices(info).map((c) => c.checked);
  const staged = { verdict: "rejected", state: { type: "draft" } } as const;
  assert.deepEqual(marked({ traits, pendingVote: staged, viewerVerdict: "approved" }), [false, false, true]);
  assert.deepEqual(marked({ traits, pendingVote: null, viewerVerdict: "waiting" }), [false, true, false]);
  assert.deepEqual(marked({ traits, pendingVote: null, viewerVerdict: null }), [false, false, false]);
  assert.deepEqual(
    voteChoices({ traits, pendingVote: null, viewerVerdict: null }).map((c) => [c.id, c.label]),
    [
      ["approved", "Approve"],
      ["waiting", "Wait for Author"],
      ["rejected", "Reject"],
    ],
  );
});

void test("posting says where the comments go and what it posts", () => {
  assert.equal(
    destination({ number: 123, platform: { host: "github.com", kind: "github", connection: { type: "ok" } } }),
    "to #123 on GitHub",
  );
  assert.equal(postLabel(1, false), "Post 1 comment");
  assert.equal(postLabel(4, false), "Post 4 comments");
  assert.equal(postLabel(4, true), "Post 4 comments and vote");
  assert.equal(postLabel(0, true), "Post vote");
});

void test("posting goes ahead only with something waiting, nothing on its way and no block", () => {
  assert.equal(canPost(2, false, undefined), true);
  assert.equal(canPost(0, false, undefined), false);
  assert.equal(canPost(2, true, undefined), false);
  assert.equal(canPost(2, false, "Sign in to post"), false);
});

void test("the submit key posts from the list, and does nothing in the vote picker, where Enter stages", () => {
  const key = (k: string, mods: { ctrlKey?: boolean; metaKey?: boolean } = {}) => ({
    key: k,
    ctrlKey: false,
    metaKey: false,
    ...mods,
  });
  assert.equal(outgoingKey(key("Enter", { ctrlKey: true }), false), "post");
  assert.equal(outgoingKey(key("Enter", { metaKey: true }), false), "post");
  assert.equal(outgoingKey(key("Enter"), false), undefined);
  assert.equal(outgoingKey(key("a", { ctrlKey: true }), false), undefined);
  assert.equal(outgoingKey(key("Enter", { ctrlKey: true }), true), undefined);
});
