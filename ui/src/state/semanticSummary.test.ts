import assert from "node:assert/strict";
import { test } from "node:test";
import type { Unavailable } from "../generated/Unavailable.ts";
import { summarize } from "./semanticSummary.ts";

const missing = (kind: Unavailable["kind"]): Unavailable => ({ kind, message: `why ${kind}`, language: null });

void test("nothing is known before the first answer", () => {
  assert.deepEqual(summarize(new Map(), new Map()), { diagnostics: undefined });
  assert.deepEqual(summarize(new Map(), new Map(), "a.rs"), { diagnostics: undefined });
});

void test("the Changes tab sums every file asked about", () => {
  const counts = new Map([
    ["a.rs", 2],
    ["b.rs", 0],
    ["c.rs", 3],
  ]);
  assert.equal(summarize(counts, new Map()).diagnostics, 5);
});

void test("a file tab counts its own file only", () => {
  const counts = new Map([
    ["a.rs", 2],
    ["b.rs", 7],
  ]);
  assert.equal(summarize(counts, new Map(), "a.rs").diagnostics, 2);
  assert.equal(summarize(counts, new Map(), "c.rs").diagnostics, undefined);
});

void test("a missing server is named, with the backend's message as the title", () => {
  const unavailable = new Map([["a.txt", missing("unsupported")]]);
  assert.deepEqual(summarize(new Map(), unavailable, "a.txt"), {
    diagnostics: undefined,
    label: "No language support",
    title: "why unsupported",
  });
  assert.equal(summarize(new Map(), unavailable, "b.rs").label, undefined);
});

void test("several files without a server are counted", () => {
  const unavailable = new Map([
    ["a.txt", missing("unsupported")],
    ["b.rs", missing("serverMissing")],
  ]);
  assert.equal(summarize(new Map(), unavailable).label, "No language support · 2 files");
});
