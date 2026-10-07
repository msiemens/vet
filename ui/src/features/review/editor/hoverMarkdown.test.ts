import assert from "node:assert/strict";
import { test } from "node:test";
import { fencedBlocks, highlightedHtml, renderMarkdown } from "./hoverMarkdown.ts";

void test("raw HTML in hover text is escaped, not passed through", () => {
  const html = renderMarkdown('<img src=x onerror="alert(1)"> <script>alert(1)</script>\n\n<div>hi</div>', []);
  assert.ok(!html.includes("<img"));
  assert.ok(!html.includes("<script"));
  assert.ok(!html.includes("<div"));
  assert.ok(html.includes("&lt;script&gt;"));
});

void test("code without highlights is escaped", () => {
  const html = renderMarkdown("```text\n<b>&</b>\n```", []);
  assert.ok(html.includes("<pre><code>&lt;b&gt;&amp;&lt;/b&gt;</code></pre>"));
});

void test("highlighted ranges are wrapped and everything is escaped", () => {
  // `fn` is a keyword (kind 0) at 0..2, then `"<"` a string (kind 1) at 5..8.
  assert.equal(
    highlightedHtml('fn f("<")', [0, 2, 0, 3, 3, 1]),
    '<span class="syn-keyword">fn</span> f(<span class="syn-string">&quot;&lt;&quot;</span>)',
  );
});

void test("highlights beyond the code and unknown kinds are ignored", () => {
  assert.equal(highlightedHtml("ab", [0, 9, 0]), '<span class="syn-keyword">ab</span>');
  assert.equal(highlightedHtml("ab", [0, 2, 42]), "ab");
});

void test("the n-th block takes the n-th highlights", () => {
  const html = renderMarkdown("```rust\nfn\n```\n\ntext\n\n```\nfn\n```\n\n```x\nfn\n```", [[0, 2, 0], [], [0, 1, 1]]);
  const blocks = html.match(/<pre><code>.*?<\/code><\/pre>/g);
  assert.deepEqual(blocks, [
    '<pre><code><span class="syn-keyword">fn</span></code></pre>',
    "<pre><code>fn</code></pre>",
    '<pre><code><span class="syn-string">f</span>n</code></pre>',
  ]);
});

void test("fenced blocks come with their highlights; indented code is not one", () => {
  const text = "    indented\n\n```rust\nfn f() {}\n```\n\n- ```py\n  pass\n  ```\n";
  assert.deepEqual(fencedBlocks(text, [[0, 2, 0], [1]]), [
    { code: "fn f() {}", highlights: [0, 2, 0] },
    { code: "pass", highlights: [1] },
  ]);
  assert.deepEqual(fencedBlocks("```\nx\n```", []), [{ code: "x", highlights: [] }]);
});

void test("links show their text without a destination; javascript: URLs go nowhere", () => {
  const html = renderMarkdown("[docs](https://example.com) and [x](javascript:alert(1)) <https://a.b>", []);
  assert.ok(!html.includes("<a"));
  assert.ok(!html.includes("href"));
  assert.ok(html.includes("docs"));
});

void test("images are not rendered", () => {
  assert.ok(!renderMarkdown("![t](https://example.com/x.png)", []).includes("<img"));
});
