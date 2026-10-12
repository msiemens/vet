import assert from "node:assert/strict";
import { test } from "node:test";
import type { Thread } from "../../../generated/Thread.ts";
import { bodyRows, cardHeight, fileViewCards, placeThreads } from "./threadPlacement.ts";

const msg = (body: string) => ({ author: "me", body, at: 0 });
const thread = (id: number, over: Partial<Thread> = {}): Thread => ({
  id,
  kind: "range",
  side: "target",
  path: "a.rs",
  original: { path: "a.rs", start: 4, len: 2 },
  location: { path: "a.rs", start: 4, len: 2 },
  targetLine: 5,
  anchor: { type: "exact", whitespaceChanged: false },
  messages: [msg("hi")],
  status: "open",
  ...over,
});

void test("threads split into file-level and inline, detached ones left out", () => {
  const threads = [
    thread(1),
    thread(2, { kind: "file", location: null, original: null, side: "target", targetLine: null }),
    thread(4, { anchor: { type: "stale", reason: "contextChanged" }, location: null, targetLine: null }),
    thread(5, { anchor: { type: "ambiguous", candidates: [] }, location: null, targetLine: null }),
    thread(7, { location: { path: "a.rs", start: 1, len: 1 }, targetLine: 1 }),
  ];
  const placed = placeThreads(threads);
  assert.deepEqual(
    placed.file.map((t) => t.id),
    [2],
  );
  // The card sits at the line the backend named, in text order.
  assert.deepEqual(
    placed.inline.map((c) => [c.thread.id, c.line]),
    [
      [7, 1],
      [1, 5],
    ],
  );
});

void test("a file's text shows the file's range threads, a base-side one at its mapped target line, and the form below the threads on its line", () => {
  const threads = [
    thread(1),
    thread(2, { side: "base", targetLine: 5 }),
    thread(3, { targetLine: 2, status: "resolved" }),
    thread(4, { path: "b.rs" }),
    thread(5, { kind: "file", location: null, original: null, targetLine: null }),
    thread(6, {
      kind: "changeset",
      path: null,
      side: null,
      location: null,
      original: null,
      targetLine: null,
      anchor: null,
    }),
    thread(7, { location: null, targetLine: null, anchor: { type: "stale", reason: "contextChanged" } }),
  ];
  const draft = {
    target: { type: "range" as const, side: "target" as const, path: "a.rs", start: 4, len: 1 },
    line: 5,
  };
  const cards = fileViewCards(threads, "a.rs", (id) => id === 3, draft);
  assert.deepEqual(
    cards.map((c) => [c.thread?.id ?? "form", c.line, c.open ?? false]),
    [
      [3, 2, true],
      [1, 5, false],
      [2, 5, false],
      ["form", 5, false],
    ],
  );
});

void test("a resolved thread is one row until expanded; heights grow with the text", () => {
  const m = { line: 20 };
  const open = thread(1, { messages: [msg("a"), msg("b\nc")] });
  const resolved = { ...open, status: "resolved" as const };
  assert.ok(cardHeight({ thread: resolved }, m) < cardHeight({ thread: thread(1) }, m));
  assert.ok(cardHeight({ thread: resolved, open: true }, m) > cardHeight({ thread: resolved }, m));
  assert.ok(cardHeight({ thread: open }, m) > cardHeight({ thread: thread(1) }, m));
  assert.ok(cardHeight({ draft: { type: "changeset" } }, m) > 0);
  assert.equal(bodyRows("a\n\nb"), 3);
  assert.equal(bodyRows("x".repeat(250)), 3);
});
