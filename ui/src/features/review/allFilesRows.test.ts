import assert from "node:assert/strict";
import { test } from "node:test";
import type { DirEntry } from "../../generated/DirEntry.ts";
import { buildAllRows } from "./allFilesRows.ts";

const dir = (name: string): DirEntry => ({ name, kind: "dir", changed: false, excluded: false });
const file = (name: string): DirEntry => ({ name, kind: "file", changed: false, excluded: false });
const shown = (listings: Record<string, DirEntry[]>, expanded: string[]) =>
  buildAllRows(new Map(Object.entries(listings)), new Set(expanded)).map(
    (r) => `${r.depth}:${r.name}${r.kind === "dir" ? "/" : ""}@${r.path}`,
  );

void test("only expanded and loaded directories show their children", () => {
  const listings = { "": [dir("a"), file("x.rs")], a: [dir("b"), file("y.rs")] };
  assert.deepEqual(shown(listings, []), ["0:a/@a", "0:x.rs@x.rs"]);
  assert.deepEqual(shown(listings, ["a"]), ["0:a/@a", "1:b/@a/b", "1:y.rs@a/y.rs", "0:x.rs@x.rs"]);
  assert.deepEqual(shown({ "": [dir("a")] }, ["a"]), ["0:a/@a"]);
});

void test("a chain of single-directory children is one row, expanded by its first path", () => {
  const listings = {
    "": [dir("crates")],
    crates: [dir("core")],
    "crates/core": [dir("src")],
    "crates/core/src": [file("lib.rs")],
  };
  assert.deepEqual(shown(listings, []), ["0:crates/core/src/@crates/core/src"]);
  assert.deepEqual(shown(listings, ["crates"]), [
    "0:crates/core/src/@crates/core/src",
    "1:lib.rs@crates/core/src/lib.rs",
  ]);
});

void test("a directory with a file beside its directory is not compacted", () => {
  const listings = { "": [dir("a")], a: [dir("b"), file("f")] };
  assert.deepEqual(shown(listings, []), ["0:a/@a"]);
});
