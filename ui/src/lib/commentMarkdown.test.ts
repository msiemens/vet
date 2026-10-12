import assert from "node:assert/strict";
import { test } from "node:test";
import { commentPreview, renderComment } from "./commentMarkdown.ts";

void test("raw HTML in a comment is shown as text, never as markup", () => {
  const html = renderComment('<img src=x onerror="steal()"> <script>steal()</script>');
  assert.doesNotMatch(html, /<img/);
  assert.doesNotMatch(html, /<script/);
  assert.match(html, /&lt;script&gt;/);
});

void test("a link may not run script, and a picture is not fetched", () => {
  assert.doesNotMatch(renderComment("[x](javascript:steal())"), /href/);
  assert.doesNotMatch(renderComment("![x](https://example.com/p.png)"), /<img/);
  assert.match(renderComment("[x](https://example.com)"), /<a href="https:\/\/example.com"/);
});

void test("a line break in a comment stays a line break", () => {
  assert.match(renderComment("one\ntwo"), /one<br>\s*two/);
});

void test("the preview is the first block's text without markup", () => {
  assert.equal(commentPreview("**Bold** and `code` and [a link](https://example.com)"), "Bold and code and a link");
  assert.equal(commentPreview("# Title\n\nbody"), "Title");
  assert.equal(commentPreview("first\nsecond\n\nthird"), "first second");
  assert.equal(commentPreview("```rs\nlet x = 1;\nlet y = 2;\n```"), "let x = 1;");
  assert.equal(commentPreview(""), "");
});
