import assert from "node:assert/strict";
import { test } from "node:test";
import type { Counts } from "./problemRows.ts";
import { type SummaryInput, originsOf, summarize } from "./problemsSummary.ts";

const counts = (over: Partial<Counts> = {}): Counts => ({
  headline: 0,
  worst: undefined,
  all: 0,
  newOnes: 0,
  existing: 0,
  unknown: 0,
  info: 0,
  ...over,
});

const input = (over: Partial<SummaryInput> = {}): SummaryInput => ({
  counts: counts(),
  comparing: false,
  unknownFiles: 0,
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

void test("once compared only the new problems are headlined", () => {
  const clean = summarize(input());
  assert.deepEqual([clean?.text, clean?.icon], ["No new problems", "ok"]);
  const some = summarize(input({ counts: counts({ headline: 1, worst: "warning", newOnes: 1, existing: 6 }) }));
  assert.deepEqual(
    [some?.text, some?.icon, some?.title],
    ["1 new problem", "warning", "1 new, 6 existing. Click to show Problems."],
  );
  assert.equal(summarize(input({ counts: counts({ headline: 9, worst: "error" }) }))?.text, "9 new problems");
});

void test("while comparing the problems are counted all together and none is called new", () => {
  const pending = summarize(input({ comparing: true, counts: counts({ headline: 11, worst: "error" }) }));
  assert.equal(pending?.text, "11 problems · comparing");
  assert.equal(pending.badgeText, "11 problems");
});

void test("files whose origin is unknown are named beside the new problems of the others", () => {
  const some = summarize(
    input({ unknownFiles: 2, counts: counts({ headline: 3, worst: "error", newOnes: 3, unknown: 4, existing: 1 }) }),
  );
  assert.equal(some?.text, "3 new problems · origin unknown in 2 files");
  assert.equal(some.badge, 3);
  assert.equal(some.title, "3 new, 1 existing, 4 of unknown origin. Click to show Problems.");
  const none = summarize(input({ unknownFiles: 1 }));
  assert.equal(none?.text, "No new problems · origin unknown in 1 file");
  assert.equal(none.icon, undefined, "not the all-clear mark");
});

void test("files' origins together are pending until every file is compared, then unknown if any could not be", () => {
  const compared = { compared: true };
  const failed = { compared: true, originUnavailable: "down" };
  assert.equal(originsOf([]), "pending");
  assert.equal(originsOf([compared, { compared: false }]), "pending");
  assert.equal(originsOf([failed, { compared: false }]), "pending");
  assert.equal(originsOf([compared, compared]), "known");
  assert.equal(originsOf([compared, failed]), "unknown");
});

void test("the badge is called new only once compared, and the tooltip tells the rest", () => {
  const some = counts({ headline: 2, worst: "error", all: 9, newOnes: 2, existing: 5, info: 2 });
  const known = summarize(input({ counts: some }));
  assert.equal(known?.badgeText, "2 new");
  assert.equal(known.title, "2 new, 5 existing, 2 information or hints. Click to show Problems.");
  const comparing = summarize(input({ comparing: true, counts: some }));
  assert.equal(comparing?.badgeText, "2 problems");
  assert.match(comparing.title, /^9 problems, 2 information or hints; which are new is not known/);
});
