import assert from "node:assert/strict";
import { test } from "node:test";
import { arrowKey } from "./foldKeys.ts";

const tree = [
  { kind: "file", depth: 1, open: true, holds: true },
  { kind: "symbol", depth: 2, open: false, holds: true },
  { kind: "symbol", depth: 3, open: false, holds: false },
  { kind: "file", depth: 1, open: false, holds: true },
  { kind: "file", depth: 1, open: false, holds: false },
];
const holds = (r: (typeof tree)[number]) => r.holds;
const open = (r: (typeof tree)[number]) => r.open;

void test("→ opens a shut row, then moves into it", () => {
  assert.deepEqual(arrowKey(tree, 1, "right", holds, open), { toggle: tree[1] });
  assert.deepEqual(arrowKey(tree, 0, "right", holds, open), { move: 1 });
  assert.equal(arrowKey(tree, 2, "right", holds, open), undefined);
  assert.deepEqual(arrowKey(tree, 3, "right", holds, open), { toggle: tree[3] });
});

void test("← shuts an open row, else moves a symbol to the row that holds it", () => {
  assert.deepEqual(arrowKey(tree, 0, "left", holds, open), { toggle: tree[0] });
  assert.deepEqual(arrowKey(tree, 2, "left", holds, open), { move: 1 });
  assert.deepEqual(arrowKey(tree, 1, "left", holds, open), { move: 0 });
  assert.equal(arrowKey(tree, 3, "left", holds, open), undefined);
});

void test("← from a symbol skips its siblings to the row that holds it", () => {
  const rows = [
    { kind: "file", depth: 1, open: true, holds: true },
    { kind: "symbol", depth: 2, open: false, holds: false },
    { kind: "symbol", depth: 2, open: false, holds: false },
  ];
  assert.deepEqual(arrowKey(rows, 2, "left", holds, open), { move: 0 });
});
