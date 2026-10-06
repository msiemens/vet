import assert from "node:assert/strict";
import { test } from "node:test";
import type { FileChange } from "../../generated/FileChange.ts";
import { buildRows, buildTree } from "./tree.ts";

const none = { lines: 0, seenLines: 0, oldBaseLines: 0, items: 0, acknowledgedItems: 0 };
const file = (path: string): FileChange => ({
  path,
  oldPath: null,
  status: "modified",
  baseLines: 0,
  targetLines: 0,
  hunks: [],
  nonText: [],
  review: { hunks: [], acknowledged: [] },
  completion: none,
});

const labels = (paths: string[], collapsed: string[] = []) =>
  buildRows(buildTree(paths.map(file)), new Set(collapsed)).map(
    (r) => `${r.depth}:${r.kind === "dir" ? r.name + "/" : r.name}`,
  );

void test("a chain of single-directory folders is one row", () => {
  assert.deepEqual(labels(["crates/vet-core/src/a.rs", "crates/vet-core/src/b.rs"]), [
    "0:crates/vet-core/src/",
    "1:a.rs",
    "1:b.rs",
  ]);
});

void test("a folder with two children is not merged with them", () => {
  assert.deepEqual(labels(["crates/vet-core/src/a.rs", "crates/vet-app/src/b.rs", "README.md"]), [
    "0:crates/",
    "1:vet-app/src/",
    "2:b.rs",
    "1:vet-core/src/",
    "2:a.rs",
    "0:README.md",
  ]);
});

void test("a folder with files of its own keeps its subfolder as a row", () => {
  assert.deepEqual(labels(["a/x.rs", "a/b/y.rs"]), ["0:a/", "1:b/", "2:y.rs", "1:x.rs"]);
});

void test("collapsing a folder hides its rows, and its descendants stay collapsed when it reopens", () => {
  const rows = buildRows(buildTree([file("a/x.rs"), file("a/b/y.rs"), file("a/c/z.rs")]), new Set());
  const a = rows.find((r) => r.kind === "dir" && r.path === "a");
  assert.ok(a?.kind === "dir");
  assert.deepEqual(a.descendants, ["a/b", "a/c"]);
  const collapsed = new Set(["a", ...a.descendants]);
  assert.deepEqual(buildRows(buildTree([file("a/x.rs")]), collapsed).length, 1);
  collapsed.delete("a");
  assert.deepEqual(
    buildRows(buildTree([file("a/x.rs"), file("a/b/y.rs"), file("a/c/z.rs")]), collapsed).map((r) => r.path),
    ["a", "a/b", "a/c", "a/x.rs"],
  );
});
