import assert from "node:assert/strict";
import { test } from "node:test";
import { trustMessage } from "./trustMessage.ts";

void test("a pull request from a fork says so, and one of an untrusted project says that", () => {
  assert.match(trustMessage({ fork: true }), /comes from a fork/);
  assert.match(trustMessage({ fork: false }), /project that is not trusted/);
});
