import assert from "node:assert/strict";
import { test } from "node:test";
import type { VersionInfo } from "../../generated/VersionInfo";
import { timelineRows } from "./timelineRows.ts";

const version = (n: number, over: Partial<VersionInfo> = {}): VersionInfo => ({
  n,
  at: n * 10,
  origin: "settled",
  tip: null,
  base: null,
  reviewedHere: false,
  files: [],
  ...over,
});

const file = (path: string, added: number, removed: number) => ({ path, delta: { added, removed } });

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

void test("a version that changed no line has no line counts", () => {
  const [row] = timelineRows([version(1, { files: [file("run.sh", 0, 0)] })]);
  assert.equal(row.lines, undefined);
});

void test("a version made by a base move names the new base instead of files", () => {
  const base = { id: "c03d55f0000", shortId: "c03d55f", subject: "", author: "", time: 0 };
  const [row] = timelineRows([version(2, { origin: "baseMoved", base })]);
  assert.equal(row.rebasedOnto, "c03d55f");
});

void test("the last reviewed version carries the eye", () => {
  const rows = timelineRows([version(1), version(2, { reviewedHere: true }), version(3)]);
  assert.deepEqual(
    rows.map((r) => r.reviewed),
    [false, true, false],
  );
});

void test("the ends of the range are marked, and the versions between them", () => {
  const rows = timelineRows(
    [1, 2, 3, 4, 5].map((n) => version(n)),
    { from: 2, to: 4, live: false },
  );
  assert.deepEqual(
    rows.map((r) => [r.n, r.band]),
    [
      [5, undefined],
      [4, "end"],
      [3, "inside"],
      [2, "end"],
      [1, undefined],
    ],
  );
});
