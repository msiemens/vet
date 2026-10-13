import assert from "node:assert/strict";
import { test } from "node:test";
import type { Unavailable } from "../generated/Unavailable.ts";
import { summarize } from "./semanticSummary.ts";

const missing = (kind: Unavailable["kind"]): Unavailable => ({
  kind,
  label: `label of ${kind}`,
  message: `why ${kind}`,
  language: null,
});

void test("nothing is known before the first answer", () => {
  assert.deepEqual(summarize(new Map(), new Map()), { answered: false });
  assert.deepEqual(summarize(new Map(), new Map(), "a.rs"), { answered: false });
});

void test("the Changes tab counts any file asked about, a file tab its own file only", () => {
  const answered = new Map([["a.rs", 1]]);
  assert.equal(summarize(answered, new Map()).answered, true);
  assert.equal(summarize(answered, new Map(), "a.rs").answered, true);
  assert.equal(summarize(answered, new Map(), "c.rs").answered, false);
});

void test("a missing server is named by the backend's label, with its message as the title", () => {
  const unavailable = new Map([["a.txt", missing("unsupported")]]);
  assert.deepEqual(summarize(new Map(), unavailable, "a.txt"), {
    answered: false,
    label: "label of unsupported",
    title: "why unsupported",
    missing: missing("unsupported"),
  });
  assert.equal(summarize(new Map(), unavailable, "b.rs").label, undefined);
});

void test("the Changes tab names no language server, however many files lack one", () => {
  const unavailable = new Map([
    ["a.txt", missing("unsupported")],
    ["b.rs", missing("serverMissing")],
  ]);
  assert.deepEqual(summarize(new Map(), unavailable), { answered: false });
});
