import assert from "node:assert/strict";
import { test } from "node:test";
import type { Thread } from "../../generated/Thread.ts";
import type { ThreadMessage } from "../../generated/ThreadMessage.ts";
import {
  candidateLabel,
  candidates,
  detachedReason,
  draftCount,
  groupThreads,
  openedBy,
  placeLabel,
  preview,
  rowTitle,
  shown,
} from "./groups.ts";

const message = (over: Partial<ThreadMessage> = {}): ThreadMessage => ({
  id: 1,
  author: "me",
  body: "hello\nworld",
  at: 0,
  mine: true,
  avatar: null,
  state: { type: "draft" },
  editedAt: null,
  conflict: null,
  ...over,
});
const thread = (id: number, over: Partial<Thread> = {}): Thread => ({
  id,
  kind: "range",
  side: "target",
  path: "a.rs",
  original: { path: "a.rs", start: 4, len: 2 },
  location: { path: "a.rs", start: 4, len: 2 },
  anchor: { type: "exact", whitespaceChanged: false },
  messages: [message()],
  status: "open",
  platform: null,
  ...over,
});
const ids = (list: Thread[]) => list.map((t) => t.id);

void test("every thread is in one group: an edit to settle, open or resolved, whatever its anchor", () => {
  const conflict = { remoteBody: "theirs", remoteAt: 1 };
  const groups = groupThreads([
    thread(1),
    thread(2, { messages: [message({ state: { type: "posted" }, conflict })], status: "resolved" }),
    thread(3, { anchor: { type: "stale", reason: "contextChanged" }, location: null }),
    thread(4, { anchor: { type: "inferred", strategy: "contextWindow", confidence: "low" } }),
    thread(5, { status: "resolved" }),
  ]);
  assert.deepEqual(ids(groups.conflicts), [2]);
  assert.deepEqual(ids(groups.open), [1, 3, 4]);
  assert.deepEqual(ids(groups.resolved), [5]);
});

void test("a long list is cut after a few rows and says how many more there are", () => {
  const list = [1, 2, 3, 4, 5];
  assert.deepEqual(shown(list, false), { rows: [1, 2, 3], more: 2 });
  assert.deepEqual(shown(list, true), { rows: list, more: 0 });
  assert.deepEqual(shown([1, 2, 3], false), { rows: [1, 2, 3], more: 0 });
});

void test("a row is titled by file and line, the file alone or the changeset", () => {
  assert.equal(rowTitle(thread(1, { path: "src/job.rs" })), "job.rs:5");
  assert.equal(rowTitle(thread(1, { kind: "file", path: "src/job.rs", location: null, original: null })), "job.rs");
  assert.equal(rowTitle(thread(1, { kind: "changeset", path: null, location: null, original: null })), "Changeset");
});

void test("on a platform a thread counts the messages that wait for the review and names who else opened it", () => {
  const platform = { posted: true, resolvable: true, resolvedBy: null };
  const theirs = thread(1, {
    platform,
    messages: [message({ author: "lena", mine: false, state: { type: "posted" } }), message({ id: 2 })],
  });
  assert.equal(draftCount(theirs), 1);
  assert.equal(openedBy(theirs), "lena");
  // Without a platform every message is only Vet's, which is not a draft of anything.
  assert.equal(draftCount(thread(2)), 0);
  assert.equal(openedBy(thread(2, { platform })), undefined);
});

void test("a group is in file order: the changeset's own first, a file's own thread before its lines in text order", () => {
  const at = (start: number) => ({ location: { path: "b.rs", start, len: 1 }, path: "b.rs" });
  const groups = groupThreads([
    thread(1, at(30)),
    thread(2, at(5)),
    thread(3, { kind: "file", path: "b.rs", location: null, original: null }),
    thread(4, { path: "a.rs" }),
    thread(5, { kind: "changeset", path: null, location: null, original: null }),
  ]);
  assert.deepEqual(ids(groups.open), [5, 4, 3, 2, 1]);
});

void test("labels name the lines, the reason and the candidates", () => {
  assert.equal(preview(thread(1)), "hello");
  assert.equal(preview(thread(1, { messages: [message({ body: "x".repeat(100) })] }), 10), "xxxxxxxxx…");
  assert.equal(placeLabel(thread(1)), "L5–6");
  // A detached range is where it was recorded.
  assert.equal(placeLabel(thread(1, { location: null, original: { path: "a.rs", start: 0, len: 1 } })), "L1");
  assert.equal(placeLabel(thread(1, { kind: "file" })), "File");
  const stale = thread(1, { anchor: { type: "stale", reason: "fileDeleted" } });
  assert.equal(detachedReason(stale), "The file was deleted");
  const where = [
    { path: "a.rs", start: 1, len: 1 },
    { path: "b.rs", start: 7, len: 3 },
  ];
  const ambiguous = thread(2, { anchor: { type: "ambiguous", candidates: where } });
  assert.equal(detachedReason(ambiguous), "2 places match");
  assert.deepEqual(candidates(ambiguous).map(candidateLabel), ["a.rs:L2", "b.rs:L8–10"]);
  assert.deepEqual(candidates(stale), []);
});
