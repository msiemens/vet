import assert from "node:assert/strict";
import { test } from "node:test";
import { signatureOf } from "./hoverContent.ts";

void test("the signature is the last code block of a hover, with its highlights", () => {
  const text = "```rust\ndemo\n```\n\n```rust\nfn delay(n: u32) -> u32\n```\n\nDocs.";
  assert.deepEqual(
    signatureOf({
      text,
      code: [
        [0, 4, 4],
        [0, 2, 0],
      ],
    }),
    {
      code: "fn delay(n: u32) -> u32",
      highlights: [0, 2, 0],
    },
  );
});

void test("an empty block is not the signature", () => {
  const text = "```rust\nfn f()\n```\n\n```\n\n```";
  assert.equal(signatureOf({ text, code: [[], []] }).code, "fn f()");
});

void test("text without a block is the signature whole, uncoloured", () => {
  assert.deepEqual(signatureOf({ text: " def f(x) ", code: [] }), { code: "def f(x)", highlights: [] });
});
