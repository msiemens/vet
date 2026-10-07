import assert from "node:assert/strict";
import { test } from "node:test";
import { keyOf } from "./symbolKey.ts";

void test("keys tell container chains apart", () => {
  assert.notEqual(keyOf({ path: "a.rs", name: ["A", "f"] }), keyOf({ path: "a.rs", name: ["B", "f"] }));
  assert.notEqual(keyOf({ path: "a.rs", name: ["A", "f"] }), keyOf({ path: "a.rs", name: ["A\0f"] }));
});
