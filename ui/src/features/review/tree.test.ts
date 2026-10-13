import assert from "node:assert/strict";
import { test } from "node:test";
import type { FileChange } from "../../generated/FileChange.ts";
import type { ModeChange } from "../../generated/ModeChange.ts";
import type { SymbolRow } from "./symbolRows.ts";
import { buildRows, buildTree, filterRows, summarize } from "./tree.ts";

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

const searchPaths = [
  "README.md",
  "ui/src/lib/Menu.svelte",
  "ui/src/review/ChangesetView.ts",
  "ui/src/review/FileTree.svelte",
  "ui/src/state/changeset.svelte.ts",
  "crates/core/src/lib.rs",
];
const found = (query: string) => {
  const result = filterRows(buildRows(buildTree(searchPaths), new Set()), query);
  return {
    shown: result.rows.map((r) => `${r.depth}:${r.name}`),
    hits: result.rows.filter((r) => result.hits.has(r)).map((r) => r.name),
  };
};

void test("a search keeps the folders above a match, expanded, and drops the rest", () => {
  assert.deepEqual(found("CSV").shown, [
    "0:ui/src",
    "1:review",
    "2:ChangesetView.ts",
    "1:state",
    "2:changeset.svelte.ts",
  ]);
  assert.deepEqual(found("CSV").hits, ["ChangesetView.ts", "changeset.svelte.ts"]);
  assert.deepEqual(found("FT").shown, ["0:ui/src", "1:review", "2:FileTree.svelte"]);
});

void test("a search finds files in several folders, in tree order", () => {
  assert.deepEqual(found("svelte").shown, [
    "0:ui/src",
    "1:lib",
    "2:Menu.svelte",
    "1:review",
    "2:FileTree.svelte",
    "1:state",
    "2:changeset.svelte.ts",
  ]);
});

void test("a folder that matches is shown, but not what is in it", () => {
  const result = found("state");
  assert.deepEqual(result.shown, ["0:ui/src", "1:state"]);
  assert.deepEqual(result.hits, ["state"]);
});

void test("a search that matches nothing shows nothing", () => {
  assert.deepEqual(found("zzz").shown, []);
});

void test("a match reports its ranges for highlighting", () => {
  const { rows, hits } = filterRows(buildRows(buildTree(searchPaths), new Set()), "FT");
  const file = rows.find((r) => r.name === "FileTree.svelte");
  assert.ok(file);
  assert.deepEqual(hits.get(file), [
    [0, 1],
    [4, 5],
  ]);
});

void test("symbols show below their file's row, and a search for one keeps the file and holders above it", () => {
  const symbol = (path: string, name: string, depth: number): SymbolRow => ({
    kind: "symbol",
    path,
    key: name,
    name,
    depth,
    symbol: {} as SymbolRow["symbol"],
    holds: false,
  });
  const symbols = (path: string, depth: number) =>
    path === "ui/src/review/FileTree.svelte"
      ? [symbol(path, "Policy", depth), symbol(path, "next_delay", depth + 1)]
      : [];
  const rows = buildRows(buildTree(searchPaths), new Set(), symbols);
  assert.deepEqual(
    rows.filter((r) => r.path === "ui/src/review/FileTree.svelte").map((r) => r.kind),
    ["file", "symbol", "symbol"],
  );
  const result = filterRows(rows, "delay");
  assert.deepEqual(
    result.rows.map((r) => `${r.depth}:${r.name}`),
    ["0:ui/src", "1:review", "2:FileTree.svelte", "3:Policy", "4:next_delay"],
  );
  assert.deepEqual(
    result.rows.filter((r) => result.hits.has(r)).map((r) => r.name),
    ["next_delay"],
  );
});

void test("a file's mode badge follows the backend's classification of its mode item", () => {
  const file = (modeChange: ModeChange | null) =>
    ({
      path: "run.sh",
      status: "modified",
      nonText: [{ kind: "mode", modeChange }],
      completion: { seenPercent: 0, oldBasePercent: 0 },
    }) as unknown as FileChange;
  assert.equal(summarize(file("gainedExecutable")).mode, "+x");
  assert.equal(summarize(file("lostExecutable")).mode, "-x");
  assert.equal(summarize(file("other")).mode, "mode");
  assert.equal(summarize({ ...file(null), nonText: [] }).mode, undefined);
});

void test("a file row carries the status letter and the progress of its summary", () => {
  const file = {
    path: "a/Cargo.toml",
    status: "modified",
    nonText: [],
    completion: { seenPercent: 50, oldBasePercent: 20 },
  } as unknown as FileChange;
  assert.deepEqual(summarize(file), { status: "M", seenPercent: 50, oldBasePercent: 20, mode: undefined });
});
