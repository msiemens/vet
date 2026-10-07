import assert from "node:assert/strict";
import { test } from "node:test";
import { wrapsByDefault } from "./softWrap.ts";

void test("prose wraps by default: Markdown, reStructuredText and plain text", () => {
  for (const path of ["README.md", "docs/guide.markdown", "docs/index.rst", "notes/todo.txt", "a/b/NOTES.TXT"])
    assert.equal(wrapsByDefault(path), true, path);
});

void test("plain text files without an extension wrap by their well-known names", () => {
  for (const path of ["README", "pkg/LICENSE", "CHANGELOG", "docs/authors"])
    assert.equal(wrapsByDefault(path), true, path);
});

void test("code, data and configuration scroll sideways by default", () => {
  for (const path of ["src/main.rs", "ui/app.ts", "Cargo.toml", "package.json", "Makefile", "index.html", "a.mdx.ts"])
    assert.equal(wrapsByDefault(path), false, path);
});

void test("only the file name counts, not the directories it is in", () => {
  assert.equal(wrapsByDefault("docs.md/main.rs"), false);
  assert.equal(wrapsByDefault("src/README/lib.rs"), false);
  assert.equal(wrapsByDefault(".md"), false); // a dotfile has no extension
});
