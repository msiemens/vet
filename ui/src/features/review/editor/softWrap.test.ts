import assert from "node:assert/strict";
import { test } from "node:test";
import { wraps } from "./softWrap.ts";

const prose = (path: string) => wraps(path, "prose");

void test("prose wraps by default: Markdown, reStructuredText and plain text", () => {
  for (const path of ["README.md", "docs/guide.markdown", "docs/index.rst", "notes/todo.txt", "a/b/NOTES.TXT"])
    assert.equal(prose(path), true, path);
});

void test("plain text files without an extension wrap by their well-known names", () => {
  for (const path of ["README", "pkg/LICENSE", "CHANGELOG", "docs/authors"]) assert.equal(prose(path), true, path);
});

void test("code, data and configuration scroll sideways by default", () => {
  for (const path of ["src/main.rs", "ui/app.ts", "Cargo.toml", "package.json", "Makefile", "index.html", "a.mdx.ts"])
    assert.equal(prose(path), false, path);
});

void test("only the file name counts, not the directories it is in", () => {
  assert.equal(prose("docs.md/main.rs"), false);
  assert.equal(prose("src/README/lib.rs"), false);
  assert.equal(prose(".md"), false); // a dotfile has no extension
});

void test("the setting can wrap every file or none", () => {
  for (const path of ["src/main.rs", "README.md"]) {
    assert.equal(wraps(path, "all"), true, path);
    assert.equal(wraps(path, "never"), false, path);
  }
});
