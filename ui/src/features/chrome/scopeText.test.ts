import assert from "node:assert/strict";
import { test } from "node:test";
import type { Base } from "../../generated/Base";
import { baseNote, scopeChip } from "./scopeText.ts";

const quiet = { live: false, pending: false };

void test("the whole changeset is All changes, with no version number", () => {
  const chip = scopeChip({ ...quiet, latest: 7 });
  assert.deepEqual([chip.label, chip.narrowed], ["All changes", false]);
});

void test("pushes are named by their own versions, a block by its ends", () => {
  assert.equal(scopeChip({ ...quiet, latest: 3, range: { from: 1, to: 2 } }).label, "Push v2");
  assert.equal(scopeChip({ ...quiet, latest: 3, range: { from: 0, to: 1 } }).label, "Push v1");
  assert.equal(scopeChip({ ...quiet, latest: 4, range: { from: 1, to: 3 } }).label, "Pushes v2–v3");
});

void test("a push says which version it is compared with, the first one the base", () => {
  assert.match(scopeChip({ ...quiet, latest: 3, range: { from: 1, to: 2 } }).tip, /compared with v1/);
  assert.match(scopeChip({ ...quiet, latest: 3, range: { from: 0, to: 1 } }).tip, /compared with the base/);
});

void test("a commit is named by its hash, and by its position until the commits are loaded", () => {
  const commit = { position: 3, of: 5, shortId: "5a6b7c8", subject: "test: cover it" };
  const chip = scopeChip({ ...quiet, commit });
  assert.deepEqual([chip.label, chip.narrowed, chip.dot], ["Commit 5a6b7c8", true, undefined]);
  assert.match(chip.tip, /5a6b7c8 test: cover it\. Commit 3 of 5/);
  assert.equal(scopeChip({ ...quiet, commit: { position: 3, of: 5 } }).label, "Commit 3");
});

void test("the dot is there while the working tree is followed, and hollow while edits wait", () => {
  assert.equal(scopeChip({ live: true, pending: false }).dot, "live");
  const waiting = scopeChip({ live: true, pending: true });
  assert.equal(waiting.dot, "pending");
  assert.match(waiting.tip, /become the next version/);
  assert.equal(scopeChip({ live: false, pending: true }).dot, undefined);
});

void test("a push is a fixed set of changes: it does not follow the working tree", () => {
  const chip = scopeChip({ live: true, pending: true, latest: 2, range: { from: 1, to: 2 } });
  assert.equal(chip.dot, undefined);
  assert.doesNotMatch(chip.tip, /become the next version/);
});

const base = (over: Partial<Base> = {}): Base => ({
  commit: { id: "a1b2c3d0", shortId: "a1b2c3d", subject: "bump", author: "", time: 0 },
  headOffBase: false,
  ...over,
});

void test("the base is said for a pull request into another branch, not for the default one", () => {
  assert.equal(baseNote(base(), "pullRequest"), undefined);
  const note = baseNote(base({ into: "release/2.1" }), "pullRequest");
  assert.deepEqual([note?.text, note?.alert], ["into release/2.1", false]);
  assert.match(note?.tip ?? "", /^Into release\/2\.1\. Branched off at a1b2c3d bump/);
});

void test("the base of a workspace is said once HEAD left it, with the alert", () => {
  assert.equal(baseNote(base(), "workingTree"), undefined);
  const note = baseNote(base({ headOffBase: true }), "workingTree");
  assert.deepEqual([note?.text, note?.alert], ["since a1b2c3d", true]);
  assert.match(note?.tip ?? "", /HEAD no longer builds on this base\.$/);
});
