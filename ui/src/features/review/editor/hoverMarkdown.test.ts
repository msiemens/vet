import assert from "node:assert/strict";
import { test } from "node:test";
import { fenceExtension } from "./fenceLanguage.ts";
import { renderMarkdown } from "./hoverMarkdown.ts";

const plain = () => undefined;

void test("fence names map to extensions, ignoring attributes and case", () => {
  assert.equal(fenceExtension("rust", "a.py"), "rs");
  assert.equal(fenceExtension("rust,ignore", "a.py"), "rs");
  assert.equal(fenceExtension("TypeScript", "a.rs"), "ts");
  assert.equal(fenceExtension("csharp", "a.rs"), "cs");
  assert.equal(fenceExtension("c#", "a.rs"), "cs");
  assert.equal(fenceExtension("py {1}", "a.rs"), "py");
});

void test("a fence without a language takes the file's, an unknown one is plain", () => {
  assert.equal(fenceExtension("", "src/lib.rs"), "src/lib.rs");
  assert.equal(fenceExtension("text", "src/lib.rs"), undefined);
});

void test("raw HTML in hover text is escaped, not passed through", () => {
  const html = renderMarkdown('<img src=x onerror="alert(1)"> <script>alert(1)</script>\n\n<div>hi</div>', plain);
  assert.ok(!html.includes("<img"));
  assert.ok(!html.includes("<script"));
  assert.ok(!html.includes("<div"));
  assert.ok(html.includes("&lt;script&gt;"));
});

void test("code is escaped when it is not highlighted", () => {
  const html = renderMarkdown("```text\n<b>&</b>\n```", plain);
  assert.ok(html.includes("<pre><code>&lt;b&gt;&amp;&lt;/b&gt;\n</code></pre>"));
});

void test("a highlighter's output is used inside the block", () => {
  const seen: string[] = [];
  const html = renderMarkdown("```rust\nfn f() {}\n```", (code, info) => {
    seen.push(info);
    return `<span>${code.trim()}</span>`;
  });
  assert.deepEqual(seen, ["rust"]);
  assert.ok(html.includes("<pre><code><span>fn f() {}</span></code></pre>"));
});

void test("links show their text without a destination; javascript: URLs go nowhere", () => {
  const html = renderMarkdown("[docs](https://example.com) and [x](javascript:alert(1)) <https://a.b>", plain);
  assert.ok(!html.includes("<a"));
  assert.ok(!html.includes("href"));
  assert.ok(html.includes("docs"));
});

void test("images are not rendered", () => {
  assert.ok(!renderMarkdown("![t](https://example.com/x.png)", plain).includes("<img"));
});
