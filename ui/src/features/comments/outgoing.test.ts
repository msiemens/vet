import assert from "node:assert/strict";
import { test } from "node:test";
import type { Thread } from "../../generated/Thread.ts";
import type { ThreadMessage } from "../../generated/ThreadMessage.ts";
import { destination, outgoingComments, outgoingPlace, postLabel, summarizeOutgoing } from "./outgoing.ts";

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
  assert.deepEqual(summarizeOutgoing(list), { count: 3, failed: 2, posting: false });
  const sending = outgoingComments([thread(1, "a.rs", 1, [message(1, posting)])]);
  assert.deepEqual(summarizeOutgoing(sending), { count: 1, failed: 0, posting: true });
  assert.deepEqual(summarizeOutgoing([]), { count: 0, failed: 0, posting: false });
});

void test("a comment says where it goes, and posting where the comments go", () => {
  assert.equal(outgoingPlace(thread(1, "src/job.rs", 11, [])), "job.rs:12");
  assert.equal(outgoingPlace(thread(1, null, 0, [])), "on the pull request");
  assert.equal(
    destination({ number: 123, platform: { host: "github.com", kind: "github", connection: { type: "ok" } } }),
    "to #123 on GitHub",
  );
  assert.equal(postLabel(1), "Post 1 comment");
  assert.equal(postLabel(4), "Post 4 comments");
});
