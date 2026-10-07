import assert from "node:assert/strict";
import { test } from "node:test";
import type { Thread } from "../generated/Thread.ts";
import { movedTitle } from "./movedTitle.ts";

const thread = (over: Partial<Thread> = {}): Thread => ({
  id: 1,
  kind: "range",
  side: "target",
  path: "a.rs",
  original: { path: "a.rs", start: 4, len: 2 },
  location: { path: "a.rs", start: 4, len: 2 },
  targetLine: 5,
  anchor: { type: "exact", whitespaceChanged: false },
  messages: [],
  status: "open",
  ...over,
});

void test("the badge says how a moved thread was found and where it was put", () => {
  const moved = thread({
    anchor: { type: "inferred", strategy: "contextWindow", confidence: "high" },
    original: { path: "a.rs", start: 1, len: 1 },
  });
  assert.equal(movedTitle(moved), "Moved: found by its surrounding lines, high confidence; was a.rs:L2");
  assert.equal(movedTitle(thread()), "");
});
