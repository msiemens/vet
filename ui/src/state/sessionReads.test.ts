import assert from "node:assert/strict";
import { test } from "node:test";
import type { SessionEvent } from "./backend.ts";
import { NO_READS, type ReadState, type Reads, merge, readsFor } from "./sessionReads.ts";

const state = (over: Partial<ReadState> = {}): ReadState => ({
  reviewShown: true,
  hasPullRequest: false,
  shownRevision: 5,
  showsChangeset: true,
  inRange: false,
  byCommit: false,
  ...over,
});

/** The names of the reads asked for, in the order the flush runs them. */
const names = (reads: Reads): string[] =>
  (["manifest", "whole", "threads", "pullRequest", "versions", "changesSince", "commits", "scopes"] as const).filter(
    (name) => reads[name],
  );

const event = (e: object) => e as SessionEvent;
const mark = (revision: number) => event({ type: "reviewChanged", project: 1, revision, update: {} });

void test("a settle reads the manifest, the whole changeset, threads, versions, changes since and the scopes", () => {
  const reads = readsFor(event({ type: "snapshotSettled", project: 1, revision: 6 }), state());

  assert.deepEqual(names(reads), ["manifest", "whole", "threads", "versions", "changesSince", "scopes"]);
  assert.deepEqual(reads.whole, { revision: 6 });
});

void test("a settle reads no versions while the review screen is not shown", () => {
  const reads = readsFor(event({ type: "snapshotSettled", project: 1, revision: 6 }), state({ reviewShown: false }));

  assert.equal(reads.versions, false);
  assert.equal(reads.changesSince, true);
});

void test("a target change reads the manifest and the whole changeset whatever the revision, and the scopes", () => {
  const reads = readsFor(event({ type: "targetChanged", project: 1 }), state());

  assert.deepEqual(names(reads), ["manifest", "whole", "scopes"]);
  assert.deepEqual(reads.whole, { revision: undefined });
});

void test("the by commit lens reads the manifest of the view it shows", () => {
  const reads = readsFor(event({ type: "lensChanged", project: 1, lens: "perCommit" }), state({ inRange: true }));

  assert.deepEqual(names(reads), ["manifest"]);
});

void test("the lens back to all changes reads the manifest, unless a range stays on view", () => {
  const lens = event({ type: "lensChanged", project: 1, lens: "squash" });

  assert.deepEqual(names(readsFor(lens, state())), ["manifest"]);
  assert.deepEqual(names(readsFor(lens, state({ inRange: true }))), []);
});

void test("changed threads read the threads, and the pull request's counts only for a pull request", () => {
  const changed = event({ type: "threadsChanged", project: 1, revision: 5 });

  assert.deepEqual(names(readsFor(changed, state())), ["threads"]);
  assert.deepEqual(names(readsFor(changed, state({ hasPullRequest: true }))), ["threads", "pullRequest"]);
});

void test("a platform status reads the pull request only for a pull request", () => {
  const status = event({ type: "platformStatus", project: 1, status: {} });

  assert.deepEqual(names(readsFor(status, state())), []);
  assert.deepEqual(names(readsFor(status, state({ hasPullRequest: true }))), ["pullRequest"]);
});

void test("a started review reads the versions and what changed since", () => {
  const reads = readsFor(event({ type: "reviewStarted", project: 1, review: {} }), state());

  assert.deepEqual(names(reads), ["versions", "changesSince"]);
  assert.equal(
    readsFor(event({ type: "reviewStarted", project: 1, review: {} }), state({ reviewShown: false })).versions,
    false,
  );
});

void test("a mark in the changeset view reads the versions and what changed since, and no commits", () => {
  assert.deepEqual(names(readsFor(mark(5), state())), ["versions", "changesSince"]);
});

void test("a mark by commit reads the commits as well", () => {
  assert.deepEqual(names(readsFor(mark(5), state({ byCommit: true }))), ["versions", "changesSince", "commits"]);
});

void test("a mark announced while a range or a commit is on view moves the commits' progress", () => {
  assert.deepEqual(names(readsFor(mark(5), state({ showsChangeset: false }))), ["versions", "changesSince", "commits"]);
});

void test("a mark of another revision than the one on view reads no commits", () => {
  assert.deepEqual(names(readsFor(mark(4), state({ byCommit: true }))), ["versions", "changesSince"]);
});

void test("the events that other stores take, and data pushes, ask the session for nothing", () => {
  for (const e of [
    { type: "changesetsChanged", project: 1 },
    { type: "editsPending", project: 1, pending: true },
    { type: "pullRequestListChanged", project: 1 },
    { type: "fileSymbols", project: 1 },
    { type: "checksChanged", project: 1 },
    { type: "serversChanged", project: 1 },
    { type: "restoreFailed", project: 1, failures: [] },
  ])
    assert.deepEqual(readsFor(event(e), state()), NO_READS, e.type);
});

void test("wants merge: a read asked for by either stays, and a target change's whole read outlasts a settle's", () => {
  const settle = readsFor(event({ type: "snapshotSettled", project: 1, revision: 6 }), state());
  const earlier = readsFor(event({ type: "snapshotSettled", project: 1, revision: 4 }), state());
  const threads = readsFor(event({ type: "threadsChanged", project: 1, revision: 5 }), state({ hasPullRequest: true }));
  const target = readsFor(event({ type: "targetChanged", project: 1 }), state());

  assert.deepEqual(names(merge(settle, threads)), [
    "manifest",
    "whole",
    "threads",
    "pullRequest",
    "versions",
    "changesSince",
    "scopes",
  ]);
  assert.deepEqual(merge(settle, earlier).whole, { revision: 6 });
  assert.deepEqual(merge(settle, target).whole, { revision: undefined });
  assert.deepEqual(merge(NO_READS, threads).whole, undefined);
});
