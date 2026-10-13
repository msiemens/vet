import assert from "node:assert/strict";
import { test } from "node:test";
import type { Counts } from "./problemRows.ts";
import { type SummaryInput, summarize } from "./problemsSummary.ts";

const counts = (over: Partial<Counts> = {}): Counts => ({
  headline: 0,
  worst: undefined,
  all: 0,
  newOnes: 0,
  existing: 0,
  ...over,
});

const input = (over: Partial<SummaryInput> = {}): SummaryInput => ({
  counts: counts(),
  origins: "known",
  files: { total: 5, answered: 5 },
  changed: 5,
  ...over,
});

void test("nothing is said while there is nothing to analyse, and files no server serves are not a zero", () => {
  assert.equal(summarize(input({ files: { total: 0, answered: 0 }, changed: 0 })), undefined);
  const none = summarize(input({ files: { total: 0, answered: 0 }, changed: 3 }));
  assert.deepEqual([none?.text, none?.badge], ["Not analysed", 0]);
});

void test("before any file has answered there is no count, not zero", () => {
  const waiting = summarize(input({ files: { total: 5, answered: 0 }, counts: counts({ headline: 4 }) }));
  assert.deepEqual([waiting?.text, waiting?.badge], ["Waiting for the language server", 0]);
});

void test("while files are still being analysed the count so far shows with the progress", () => {
  const partway = summarize(
    input({ files: { total: 5, answered: 2 }, counts: counts({ headline: 3, worst: "error" }) }),
  );
  assert.deepEqual([partway?.text, partway?.badge], ["Analysing 2 of 5 files", 3]);
});

void test("with origins known only the new problems are headlined", () => {
  const clean = summarize(input());
  assert.deepEqual([clean?.text, clean?.icon], ["No new problems", "ok"]);
  const some = summarize(input({ counts: counts({ headline: 1, worst: "warning", newOnes: 1, existing: 6 }) }));
  assert.deepEqual(
    [some?.text, some?.icon, some?.title],
    ["1 new problem", "warning", "1 new, 6 existing. Click to show Problems."],
  );
  assert.equal(summarize(input({ counts: counts({ headline: 9, worst: "error" }) }))?.text, "9 new problems");
});

void test("without known origins the problems are counted all together and no claim is made about new", () => {
  const pending = summarize(input({ origins: "pending", counts: counts({ headline: 11, worst: "error" }) }));
  assert.equal(pending?.text, "11 problems · comparing");
  const unknown = summarize(input({ origins: "unknown", counts: counts({ headline: 11, worst: "error" }) }));
  assert.equal(unknown?.text, "11 problems");
  assert.equal(summarize(input({ origins: "unknown" }))?.text, "No problems");
});
