import assert from "node:assert/strict";
import { test } from "node:test";
import type { FileChange } from "../../generated/FileChange.ts";
import { buildRows, buildTree, summarize } from "./tree.ts";

const labels = (paths: string[], collapsed: string[] = []) =>
  buildRows(buildTree(paths), new Set(collapsed)).map((r) => `${r.depth}:${r.kind === "dir" ? r.name + "/" : r.name}`);

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
  const paths = ["a/x.rs", "a/b/y.rs", "a/c/z.rs"];
  const a = buildRows(buildTree(paths), new Set()).find((r) => r.kind === "dir" && r.path === "a");
  assert.ok(a?.kind === "dir");
  assert.deepEqual(a.descendants, ["a/b", "a/c"]);
  const collapsed = new Set(["a", ...a.descendants]);
  assert.equal(buildRows(buildTree(paths), collapsed).length, 1);
  collapsed.delete("a");
  assert.deepEqual(
    buildRows(buildTree(paths), collapsed).map((r) => r.path),
    ["a", "a/b", "a/c", "a/x.rs"],
  );
});

void test("a file's progress comes from its summary, not from the tree", () => {
  const file = {
    path: "a/Cargo.toml",
    status: "modified",
    nonText: [],
    completion: { lines: 10, seenLines: 5, oldBaseLines: 2, items: 0, acknowledgedItems: 0 },
  } as unknown as FileChange;
  assert.deepEqual(summarize(file), { status: "M", seenPct: 50, oldPct: 20, mode: undefined });
});
