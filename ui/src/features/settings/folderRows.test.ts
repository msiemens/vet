import assert from "node:assert/strict";
import { test } from "node:test";
import { folderRows } from "./folderRows.ts";

const listed = new Map([
  ["", ["crates", "ui"]],
  ["crates", ["vet-app", "vet-core"]],
  ["ui", []],
]);

void test("only the folders under an open one are listed, nested", () => {
  assert.deepEqual(
    folderRows(listed, new Set(["crates"])).map((r) => [r.path, r.depth, r.open]),
    [
      ["crates", 0, true],
      ["crates/vet-app", 1, false],
      ["crates/vet-core", 1, false],
      ["ui", 0, undefined],
    ],
  );
});

void test("a folder not listed yet can be opened and one that holds no folder cannot", () => {
  const rows = folderRows(listed, new Set());
  assert.deepEqual(
    rows.map((r) => [r.path, r.open]),
    [
      ["crates", false],
      ["ui", undefined],
    ],
  );
});
