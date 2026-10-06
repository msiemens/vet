import assert from "node:assert/strict";
import { test } from "node:test";
import type { Thread } from "../../../generated/Thread.ts";
import {
  bodyRows,
  cardHeight,
  draftTitle,
  movedTitle,
  placeThreads,
  selectedLines,
  targetLine,
} from "./threadPlacement.ts";

const hunk = (baseStart: number, baseLen: number, targetStart: number, targetLen: number) => ({
  baseStart,
  baseLen,
  targetStart,
  targetLen,
});
const msg = (body: string) => ({ author: "me", body, at: 0 });
const thread = (id: number, over: Partial<Thread> = {}): Thread => ({
  id,
  kind: "range",
  side: "target",
  path: "a.rs",
  original: { path: "a.rs", start: 4, len: 2 },
  location: { path: "a.rs", start: 4, len: 2 },
  anchor: { type: "exact", whitespaceChanged: false },
  messages: [msg("hi")],
  status: "open",
  ...over,
});

void test("a base line faces the hunk it was changed in, or where it shifted to", () => {
  // Lines 10..12 of the base became one target line at 10; two lines were added at 20.
  const hunks = [hunk(10, 2, 10, 1), hunk(18, 0, 17, 2)];
  assert.equal(targetLine(hunks, "target", 7), 7);
  assert.equal(targetLine(hunks, "base", 5), 5);
  assert.equal(targetLine(hunks, "base", 11), 10); // inside the first hunk
  assert.equal(targetLine(hunks, "base", 14), 13); // one line fewer after it
  assert.equal(targetLine(hunks, "base", 30), 31); // and one line more after both
});

void test("a pure deletion's base lines face the line above it", () => {
  const hunks = [hunk(5, 3, 5, 0)];
  assert.equal(targetLine(hunks, "base", 6), 5);
  assert.equal(targetLine(hunks, "base", 0), 0);
  assert.equal(targetLine([hunk(0, 2, 0, 0)], "base", 1), 0);
});

void test("threads split into file-level and inline, detached ones and other files left out", () => {
  const threads = [
    thread(1),
    thread(2, { kind: "file", location: null, original: null, side: "target" }),
    thread(3, { path: "b.rs" }),
    thread(4, { anchor: { type: "stale", reason: "contextChanged" }, location: null }),
    thread(5, { anchor: { type: "ambiguous", candidates: [] }, location: null }),
    thread(6, { kind: "changeset", path: null, location: null, original: null, side: null, anchor: null }),
    thread(7, { location: { path: "a.rs", start: 1, len: 1 } }),
  ];
  const placed = placeThreads("a.rs", [], 50, threads);
  assert.deepEqual(
    placed.file.map((t) => t.id),
    [2],
  );
  // The card sits below the last line of the range, in text order.
  assert.deepEqual(
    placed.inline.map((c) => [c.thread.id, c.line]),
    [
      [7, 1],
      [1, 5],
    ],
  );
});

void test("a card never sits past the last line", () => {
  const placed = placeThreads("a.rs", [], 3, [thread(1, { location: { path: "a.rs", start: 9, len: 4 } })]);
  assert.equal(placed.inline[0].line, 2);
});

void test("a resolved thread is one row until expanded; heights grow with the text", () => {
  const m = { line: 20 };
  const open = thread(1, { messages: [msg("a"), msg("b\nc")] });
  const resolved = { ...open, status: "resolved" as const };
  assert.equal(cardHeight({ thread: resolved }, m), 14 + 20);
  assert.ok(cardHeight({ thread: resolved, open: true }, m) > cardHeight({ thread: resolved }, m));
  assert.ok(cardHeight({ thread: open }, m) > cardHeight({ thread: thread(1) }, m));
  assert.ok(cardHeight({ draft: { type: "changeset" } }, m) > 0);
  assert.equal(bodyRows("a\n\nb"), 3);
  assert.equal(bodyRows("x".repeat(250)), 3);
});

void test("a selection ending at the start of a line does not include it", () => {
  assert.deepEqual(selectedLines(3, 5, false), { start: 3, len: 3 });
  assert.deepEqual(selectedLines(3, 5, true), { start: 3, len: 2 });
  assert.deepEqual(selectedLines(3, 3, true), { start: 3, len: 1 });
});

void test("titles say where a thread moved from and what a draft is about", () => {
  const moved = thread(1, {
    anchor: { type: "inferred", strategy: "contextWindow", confidence: "high" },
    original: { path: "a.rs", start: 1, len: 1 },
  });
  assert.equal(movedTitle(moved), "Moved: found by its surrounding lines, high confidence; was a.rs:L2");
  assert.equal(movedTitle(thread(1)), "");
  assert.equal(draftTitle({ type: "range", side: "target", path: "a.rs", start: 4, len: 2 }), "Comment on L5–6");
  assert.equal(draftTitle({ type: "range", side: "base", path: "a.rs", start: 4, len: 1 }), "Comment on base L5");
  assert.equal(draftTitle({ type: "file", side: "target", path: "a.rs" }), "Comment on this file");
});
