import assert from "node:assert/strict";
import { test } from "node:test";
import { parsePosition } from "./savedPosition.ts";

void test("a stored position comes back as it was written", () => {
  const position = { path: "src/a.rs", offset: -12.5 };
  assert.deepEqual(parsePosition(JSON.stringify(position)), position);
});

void test("text that is not a position is none", () => {
  for (const text of [
    undefined,
    "",
    "{",
    "null",
    "3",
    '{"path":"a"}',
    '{"path":1,"offset":2}',
    '{"path":"a","offset":"2"}',
  ]) {
    assert.equal(parsePosition(text), undefined, String(text));
  }
});
