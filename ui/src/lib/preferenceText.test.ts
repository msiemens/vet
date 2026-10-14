import assert from "node:assert/strict";
import { test } from "node:test";
import { decode, encode } from "./preferenceText.ts";

const numbers = (json: unknown) => (Array.isArray(json) && json.every((n) => typeof n === "number") ? json : undefined);

void test("a kept value is read back, and damaged or foreign text gives the fallback", () => {
  assert.deepEqual(decode("[1,2]", numbers, []), [1, 2]);
  assert.deepEqual(decode(undefined, numbers, [7]), [7]);
  assert.deepEqual(decode("{not json", numbers, [7]), [7]);
  assert.deepEqual(decode('["a"]', numbers, [7]), [7]);
  assert.deepEqual(decode("null", numbers, [7]), [7]);
});

void test("only a value that differs from the default is kept", () => {
  assert.equal(encode(true, false), "true");
  assert.equal(encode(false, false), undefined);
  assert.equal(encode({}, {}), undefined);
  assert.equal(encode({ a: 1 }, {}), '{"a":1}');
  assert.equal(encode(undefined, undefined), undefined);
  assert.equal(encode(120, undefined), "120");
  assert.equal(encode(undefined, 120), undefined);
});
