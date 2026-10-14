import assert from "node:assert/strict";
import { test } from "node:test";
import type { CommitInfo } from "../../generated/CommitInfo";
import type { VersionInfo } from "../../generated/VersionInfo";
import { baseRow, timelineRows, timelineSummary } from "./timelineRows.ts";

const version = (n: number, over: Partial<VersionInfo> = {}): VersionInfo => ({
  n,
  at: n * 10,
  origin: "settled",
  available: true,
  tip: null,
  base: null,
  reviewedHere: false,
  files: [],
  ...over,
});

const file = (path: string, added: number, removed: number) => ({ path, delta: { added, removed } });
const commit = (shortId: string): CommitInfo => ({ id: `${shortId}000`, shortId, subject: "", author: "", time: 0 });
const five = [1, 2, 3, 4, 5].map((n) => version(n));

void test("rows run newest first and name each version's files by their short names", () => {
  const rows = timelineRows([
    version(1, { files: [file("src/sync/retry.rs", 4, 2)] }),
    version(2, { files: [file("src/sync/retry.rs", 3, 1), file("src/sync/job.rs", 0, 0)] }),
  ]);

  assert.deepEqual(
    rows.map((r) => [r.n, r.files, r.lines]),
    [
      [2, "retry.rs, job.rs", { added: 3, removed: 1 }],
      [1, "retry.rs", { added: 4, removed: 2 }],
    ],
  );
});

void test("a version whose commits are gone is a row that cannot be chosen", () => {
  const rows = timelineRows([version(1), version(2, { available: false }), version(3)]);
  assert.deepEqual(
    rows.map((r) => [r.n, r.available]),
    [
      [3, true],
      [2, false],
      [1, true],
    ],
  );
});

void test("a version that changed no line has no line counts", () => {
  const [row] = timelineRows([version(1, { files: [file("run.sh", 0, 0)] })]);
  assert.equal(row.lines, undefined);
});

void test("a pushed version names its commit for its checks", () => {
  const tip = commit("e91a4d0");
  const [row] = timelineRows([version(1, { tip })]);
  assert.equal(row.commit, "e91a4d0000");
});

void test("the whole changeset shows every push and is compared with the base", () => {
  const rows = timelineRows(five);
  assert.deepEqual(
    rows.map((r) => [r.n, r.shown, r.ring]),
    [5, 4, 3, 2, 1].map((n) => [n, true, false]),
  );
  assert.equal(baseRow(five, commit("a1b2c3d"))?.ring, true);
});

void test("one push shows only itself and is compared with the version before it", () => {
  const range = { from: 2, to: 3 };
  const rows = timelineRows(five, { range });
  assert.deepEqual(
    rows.map((r) => [r.n, r.shown, r.ring]),
    [
      [5, false, false],
      [4, false, false],
      [3, true, false],
      [2, false, true],
      [1, false, false],
    ],
  );
});

void test("a block shows each of its pushes, and the first push is compared with the base", () => {
  const range = { from: 0, to: 2 };
  const rows = timelineRows(five, { range });
  assert.deepEqual(
    rows.map((r) => [r.n, r.shown, r.ring]),
    [
      [5, false, false],
      [4, false, false],
      [3, false, false],
      [2, true, false],
      [1, true, false],
    ],
  );
  assert.equal(baseRow(five, commit("a1b2c3d"), { range })?.ring, true);
});

void test("a commit on view marks no push and nothing to compare with", () => {
  const rows = timelineRows(five, { byCommit: true });
  assert.deepEqual(
    rows.map((r) => r.shown || r.ring),
    [false, false, false, false, false],
  );
  assert.equal(baseRow(five, commit("a1b2c3d"), { byCommit: true })?.ring, false);
});

void test("the header names the pushes shown, and nothing before a review", () => {
  const says = (shown: Parameters<typeof timelineSummary>[0]) => timelineSummary(shown, 3);
  assert.match(says({ range: { from: 1, to: 2 } }), /v2/);
  assert.doesNotMatch(says({ range: { from: 1, to: 2 } }), /v1|v3/);
  assert.match(says({ range: { from: 0, to: 2 } }), /v1.*v2/);
  assert.notEqual(says({}), says({ range: { from: 0, to: 2 } }));
  assert.match(says({ byCommit: true, range: { from: 0, to: 2 } }), /commit/i);
  assert.equal(timelineSummary({}, undefined), "");
});

void test("the whole changeset is compared with the base now, a range from the start with the first version's", () => {
  const first = version(1, { base: commit("a1b2c3d") });
  const rebased = commit("f00ba44");
  assert.equal(baseRow([first], rebased)?.commit.shortId, "f00ba44");
  assert.equal(baseRow([first], rebased, { range: { from: 0, to: 1 } })?.commit.shortId, "a1b2c3d");
});

void test("the base is shown once it is known, and only a range from the start is compared with it", () => {
  assert.equal(baseRow([version(1)], undefined), undefined);
  assert.equal(baseRow([version(1)], commit("a1b2c3d"))?.commit.shortId, "a1b2c3d");
  assert.equal(baseRow(five, commit("a1b2c3d"), { range: { from: 2, to: 3 } })?.ring, false);
});
