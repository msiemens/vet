import assert from "node:assert/strict";
import { test } from "node:test";
import { noServerText } from "./hoverContent.ts";

void test("a file no server handles is named by its extension", () => {
  assert.equal(noServerText("ui/src/App.svelte"), "No language server for .svelte files.");
  assert.equal(noServerText("archive.tar.gz"), "No language server for .gz files.");
});

void test("a file without an extension, or a dotfile, is named by its name", () => {
  assert.equal(noServerText("tools/Makefile"), "No language server for Makefile.");
  assert.equal(noServerText(".editorconfig"), "No language server for .editorconfig.");
  assert.equal(noServerText("notes.d/README"), "No language server for README.");
});
