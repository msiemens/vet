import assert from "node:assert/strict";
import { test } from "node:test";
import { basename, compactChain, dirname, joinPath, pathsBelow, repoName, splitPath } from "./paths.ts";

void test("a path splits at its last slash", () => {
  assert.equal(basename("a/b/c.rs"), "c.rs");
  assert.equal(dirname("a/b/c.rs"), "a/b");
  assert.equal(basename("c.rs"), "c.rs");
  assert.equal(dirname("c.rs"), "");
  assert.equal(joinPath("", "a"), "a");
  assert.equal(joinPath("a", "b"), "a/b");
});

void test("a path shows as its folders, trailing slash kept, and its file name", () => {
  assert.deepEqual(splitPath("a/b/c.rs"), { dir: "a/b/", name: "c.rs" });
  assert.deepEqual(splitPath("c.rs"), { dir: "", name: "c.rs" });
  assert.deepEqual(splitPath("/c.rs"), { dir: "/", name: "c.rs" });
});

void test("a Windows path splits at its last backslash, and a folder keeps its trailing separator with its name", () => {
  assert.deepEqual(splitPath("C:\\work\\vet\\a.rs"), { dir: "C:\\work\\vet\\", name: "a.rs" });
  assert.deepEqual(splitPath("C:\\work\\vet\\"), { dir: "C:\\work\\", name: "vet\\" });
  assert.deepEqual(splitPath("/home/me/logs/"), { dir: "/home/me/", name: "logs/" });
  assert.deepEqual(splitPath("/home/me\\mixed/a"), { dir: "/home/me\\mixed/", name: "a" });
});

void test("a root has no folders to dim", () => {
  assert.deepEqual(splitPath("/"), { dir: "", name: "/" });
  assert.deepEqual(splitPath("C:\\"), { dir: "", name: "C:\\" });
  assert.deepEqual(splitPath(""), { dir: "", name: "" });
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

void test("the paths below a directory are those inside it at any depth, not those of a sibling that shares its prefix", () => {
  const paths = ["src/a.rs", "src/deep/b.rs", "src2/c.rs", "src", "top.rs"];
  assert.deepEqual(pathsBelow(paths, "src"), ["src/a.rs", "src/deep/b.rs"]);
  assert.deepEqual(pathsBelow(paths, "src/deep"), ["src/deep/b.rs"]);
  assert.deepEqual(pathsBelow(paths, "nope"), []);
});

void test("the repository name is the last non-empty segment, on either separator", () => {
  assert.equal(repoName("/home/me/vet"), "vet");
  assert.equal(repoName("C:\\work\\vet\\"), "vet");
  assert.equal(repoName("/"), "/");
});
