import assert from "node:assert/strict";
import { test } from "node:test";
import { basename, compactChain, dirname, joinPath, repoName } from "./paths.ts";

void test("a path splits at its last slash", () => {
  assert.equal(basename("a/b/c.rs"), "c.rs");
  assert.equal(dirname("a/b/c.rs"), "a/b");
  assert.equal(basename("c.rs"), "c.rs");
  assert.equal(dirname("c.rs"), "");
  assert.equal(joinPath("", "a"), "a");
  assert.equal(joinPath("a", "b"), "a/b");
});

void test("a chain stops at the first directory with anything but one directory", () => {
  const only: Record<string, string> = { a: "a/b", "a/b": "a/b/c" };
  assert.deepEqual(
    compactChain("a", "a", (p) => only[p]),
    { path: "a/b/c", name: "a/b/c" },
  );
  assert.deepEqual(
    compactChain("x", "x", (p) => only[p]),
    { path: "x", name: "x" },
  );
});

void test("the repository name is the last non-empty segment, on either separator", () => {
  assert.equal(repoName("/home/me/vet"), "vet");
  assert.equal(repoName("C:\\work\\vet\\"), "vet");
  assert.equal(repoName("/"), "/");
});
