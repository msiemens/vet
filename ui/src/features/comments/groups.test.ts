import assert from "node:assert/strict";
import { test } from "node:test";
import type { Thread } from "../../generated/Thread.ts";
import { candidateLabel, candidates, detachedReason, groupThreads, placeLabel, preview } from "./groups.ts";

const thread = (id: number, over: Partial<Thread> = {}): Thread => ({
  id,
  kind: "range",
  side: "target",
  path: "a.rs",
  original: { path: "a.rs", start: 4, len: 2 },
  location: { path: "a.rs", start: 4, len: 2 },
  anchor: { type: "exact", whitespaceChanged: false },
  messages: [{ author: "me", body: "hello\nworld", at: 0 }],
  status: "open",
  ...over,
});
const ids = (list: Thread[]) => list.map((t) => t.id);

void test("detached and moved threads leave their file's group", () => {
  const groups = groupThreads([
    thread(1),
    thread(2, { anchor: { type: "stale", reason: "contextChanged" }, location: null }),
    thread(3, { anchor: { type: "inferred", strategy: "contextWindow", confidence: "low" } }),
    thread(4, { anchor: { type: "ambiguous", candidates: [{ path: "a.rs", start: 1, len: 1 }] }, location: null }),
    thread(5, { kind: "changeset", path: null, location: null, original: null, side: null, anchor: null }),
  ]);
  assert.deepEqual(ids(groups.detached), [2, 4]);
  assert.deepEqual(ids(groups.moved), [3]);
  assert.deepEqual(ids(groups.changeset), [5]);
  assert.deepEqual(
    groups.files.map((f) => [f.path, ids(f.threads)]),
    [["a.rs", [1]]],
  );
});

void test("files are sorted by path, a file's own thread first, then lines in text order", () => {
  const at = (start: number) => ({ location: { path: "b.rs", start, len: 1 }, path: "b.rs" });
  const groups = groupThreads([
    thread(1, at(30)),
    thread(2, at(5)),
    thread(3, { kind: "file", path: "b.rs", location: null, original: null }),
    thread(4, { path: "a.rs" }),
  ]);
  assert.deepEqual(
    groups.files.map((f) => [f.path, ids(f.threads)]),
    [
      ["a.rs", [4]],
      ["b.rs", [3, 2, 1]],
    ],
  );
});

void test("labels name the lines, the reason and the candidates", () => {
  assert.equal(preview(thread(1)), "hello");
  assert.equal(preview(thread(1, { messages: [{ author: "me", body: "x".repeat(100), at: 0 }] }), 10), "xxxxxxxxx…");
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
