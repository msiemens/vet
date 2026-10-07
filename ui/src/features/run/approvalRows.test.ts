import assert from "node:assert/strict";
import { test } from "node:test";
import type { ApprovalFile } from "../../generated/ApprovalFile.ts";
import { MAX_ROWS, approvalRows } from "./approvalRows.ts";

const file = (over: Partial<ApprovalFile>): ApprovalFile => ({
  path: "a",
  change: "modified",
  base: "",
  target: "",
  hunks: [],
  delta: { added: 0, removed: 0 },
  ...over,
});
const text = (n: number) => Array.from({ length: n }, (_, i) => `l${i + 1}`).join("\n") + "\n";
const show = (rows: ReturnType<typeof approvalRows>) =>
  rows?.rows.map((r) => (r.kind === "gap" ? "~" : `${r.kind[0]}${r.n}`)).join(" ");

void test("a changed line shows its context and the old line", () => {
  const base = text(10);
  const target = base.replace("l5", "L5");
  const rows = approvalRows(
    file({ base, target, hunks: [{ baseStart: 4, baseLen: 1, targetStart: 4, targetLen: 1 }] }),
  );
  assert.equal(show(rows), "~ c3 c4 d5 a5 c6 c7 ~");
});

void test("hunks close together share their context, distant ones are separated", () => {
  const near = approvalRows(
    file({
      base: text(12),
      target: text(12),
      hunks: [
        { baseStart: 1, baseLen: 1, targetStart: 1, targetLen: 1 },
        { baseStart: 5, baseLen: 1, targetStart: 5, targetLen: 1 },
      ],
    }),
  );
  assert.equal(show(near), "c1 d2 a2 c3 c4 c5 d6 a6 c7 c8 ~");
  const far = approvalRows(
    file({
      base: text(30),
      target: text(30),
      hunks: [
        { baseStart: 1, baseLen: 1, targetStart: 1, targetLen: 1 },
        { baseStart: 20, baseLen: 1, targetStart: 20, targetLen: 1 },
      ],
    }),
  );
  assert.equal(show(far), "c1 d2 a2 c3 c4 ~ c19 c20 d21 a21 c22 c23 ~");
});

void test("an added file is all additions", () => {
  const rows = approvalRows(
    file({
      change: "added",
      base: null,
      target: text(2),
      hunks: [{ baseStart: 0, baseLen: 0, targetStart: 0, targetLen: 2 }],
    }),
  );
  assert.equal(show(rows), "a1 a2");
});

void test("a side without text only matters for a modified file", () => {
  assert.equal(approvalRows(file({ target: null })), undefined);
  assert.deepEqual(approvalRows(file({ change: "removed", target: null, base: "x\n" }))?.rows, []);
});

void test("long diffs are cut", () => {
  const n = MAX_ROWS + 50;
  const rows = approvalRows(
    file({
      change: "added",
      base: null,
      target: text(n),
      hunks: [{ baseStart: 0, baseLen: 0, targetStart: 0, targetLen: n }],
    }),
  );
  assert.deepEqual([rows?.rows.length, rows?.truncated], [MAX_ROWS, true]);
});
