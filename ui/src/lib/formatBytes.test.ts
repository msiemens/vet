import assert from "node:assert/strict";
import { test } from "node:test";
import { formatBytes } from "./formatBytes.ts";

void test("sizes switch to a binary unit at 1024", () => {
  assert.equal(formatBytes(0), "0 B");
  assert.equal(formatBytes(1023), "1023 B");
  assert.equal(formatBytes(1536), "1.5 KiB");
  assert.equal(formatBytes(1024 * 1024), "1.0 MiB");
  assert.equal(formatBytes(1024 ** 3), "1.0 GiB");
  assert.equal(formatBytes(1.5 * 1024 ** 3), "1.5 GiB");
  assert.equal(formatBytes(1023.9 * 1024 ** 2), "1023.9 MiB");
});
