import assert from "node:assert/strict";
import { test } from "node:test";
import { draftPatterns, matchText, rootPath, withEdit } from "./filterDraft.ts";

void test("a draft is the list with the edits applied and the add field's text last", () => {
  assert.deepEqual(draftPatterns(["*.rs", "docs/**"], { edits: { 1: "docs/*" }, adding: "  *.md " }), [
    "*.rs",
    "docs/*",
    "*.md",
  ]);
  assert.deepEqual(draftPatterns(["*.rs"], { edits: {}, adding: "   " }), ["*.rs"]);
});

void test("an edited row is replaced and an emptied one dropped", () => {
  assert.deepEqual(withEdit(["a", "b", "c"], 1, " B "), ["a", "B", "c"]);
  assert.deepEqual(withEdit(["a", "b", "c"], 1, "  "), ["a", "c"]);
});

void test("a typed root loses its slashes at both ends", () => {
  assert.equal(rootPath(" /crates/vet-app// "), "crates/vet-app");
  assert.equal(rootPath("/"), "");
});

void test("what a pattern matches is counted in files and lines, singular for one", () => {
  assert.equal(matchText({ files: 1, lines: 1 }), "1 file · 1 line");
  assert.equal(matchText({ files: 2, lines: 0 }), "2 files · 0 lines");
});
