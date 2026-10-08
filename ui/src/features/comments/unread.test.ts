import assert from "node:assert/strict";
import { test } from "node:test";
import type { ConversationEntry } from "../../generated/ConversationEntry.ts";
import type { Thread } from "../../generated/Thread.ts";
import type { ThreadMessage } from "../../generated/ThreadMessage.ts";
import { seenKey, seenNow, unreadCount } from "./unread.ts";

const entry = (at: number, kind: ConversationEntry["kind"]): ConversationEntry => ({
  id: `E${at}`,
  author: "lena",
  avatar: null,
  at,
  kind,
  body: "",
});
const message = (at: number, mine = false): ThreadMessage => ({
  id: at,
  author: mine ? "ada" : "lena",
  body: "",
  at,
  mine,
  avatar: null,
  state: { type: "posted" },
  editedAt: null,
  conflict: null,
});
const thread = (kind: Thread["kind"], messages: ThreadMessage[]): Thread => ({
  id: messages[0]?.at ?? 0,
  kind,
  side: kind === "changeset" ? null : "target",
  path: kind === "changeset" ? null : "a.rs",
  original: null,
  location: null,
  targetLine: null,
  anchor: null,
  messages,
  status: "open",
  platform: { posted: true, resolvable: true, resolvedBy: null },
});

const feed = {
  conversation: [
    entry(100, { type: "comment", mine: false }),
    entry(200, { type: "review", verdict: "approved", mine: false }),
    entry(300, { type: "comment", mine: true }),
    entry(400, { type: "review", verdict: "commented", mine: true }),
  ],
};

void test("others' conversation comments and reviews since the tool window was last open are unread, the viewer's are not", () => {
  assert.equal(unreadCount(feed, [], undefined), 2, "never opened: all of them");
  assert.equal(unreadCount(feed, [], 150), 1);
  assert.equal(unreadCount(feed, [], 200), 0);
});

void test("replies to threads on the pull request count, comments on lines do not", () => {
  const threads = [
    thread("changeset", [message(100, true), message(250), message(260, true)]),
    thread("range", [message(500)]),
    thread("file", [message(600)]),
  ];
  assert.equal(unreadCount(undefined, threads, 200), 1);
  assert.equal(unreadCount(feed, threads, 150), 2);
});

void test("opening the tool window sees up to now, or to the newest comment where the platform's clock is ahead", () => {
  const threads = [thread("changeset", [message(900)])];
  assert.equal(seenNow(feed, threads, 1_000), 1_000);
  assert.equal(seenNow(feed, threads, 800), 900);
  assert.equal(unreadCount(feed, threads, seenNow(feed, threads, 800)), 0);
});

void test("the time is kept per pull request, and a changeset that is none has no unread comments to keep", () => {
  assert.notEqual(
    seenKey({ type: "pullRequest", remote: "github.com/acme/app", number: 7 }),
    seenKey({ type: "pullRequest", remote: "github.com/acme/app", number: 8 }),
  );
  assert.equal(seenKey({ type: "workingTree" }), undefined);
  assert.equal(seenKey(undefined), undefined);
});
