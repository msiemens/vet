import assert from "node:assert/strict";
import { test } from "node:test";
import type { Place } from "./foundPlace.ts";
import { changedSince, diffRuns, isStale, slotFor } from "./findRuns.ts";

void test("a result replaces the first tab that is not pinned, opens a new one when all are pinned, and reruns the same query", () => {
  const sets = [
    { key: "a", pinned: true },
    { key: "b", pinned: false },
    { key: "c", pinned: true },
  ];
  assert.deepEqual(slotFor(sets, "new"), { kind: "replace", index: 1 });
  assert.deepEqual(slotFor(sets, "c"), { kind: "rerun", index: 2 });
  assert.deepEqual(slotFor(sets, "b"), { kind: "rerun", index: 1 });
  assert.deepEqual(slotFor([sets[0], sets[2]], "new"), { kind: "open" });
  assert.deepEqual(slotFor([], "new"), { kind: "open" });
});

const at = (path: string, text: string, col = 0): Place =>
  ({ id: `${path}:${text}`, file: { path }, text, col }) as Place;

void test("a rerun counts what is new and what is gone, a place that only changed line being neither", () => {
  const before = [at("a.rs", "f(1)"), at("a.rs", "g(2)"), at("b.rs", "f(3)")];
  const after = [at("a.rs", "f(1)"), at("b.rs", "f(3)"), at("c.rs", "f(4)")];
  const { fresh, gone } = diffRuns(before, after);
  assert.deepEqual([...fresh], ["c.rs:f(4)"]);
  assert.equal(gone, 1);
});

void test("the files changed since a set ran are those of the versions after it", () => {
  const versions = [
    { n: 1, files: [{ path: "a.rs" }] },
    { n: 2, files: [{ path: "b.rs" }, { path: "c.rs" }] },
    { n: 3, files: [{ path: "b.rs" }] },
  ];
  assert.deepEqual([...changedSince(versions, 1)].sort(), ["b.rs", "c.rs"]);
  assert.deepEqual([...changedSince(versions, 3)], []);
  assert.deepEqual([...changedSince(versions, undefined)], []);
});

void test("a finished set of the changeset is stale on another revision; a commit's set, a running one and an unknown revision are not", () => {
  const set = { done: true, commit: false, revision: 4 };
  assert.equal(isStale(set, 5), true);
  assert.equal(isStale(set, 4), false);
  assert.equal(isStale({ ...set, commit: true }, 5), false);
  assert.equal(isStale({ ...set, done: false }, 5), false);
  assert.equal(isStale(set, undefined), false);
  assert.equal(isStale({ ...set, revision: undefined }, 5), false);
});
