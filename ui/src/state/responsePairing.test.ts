import assert from "node:assert/strict";
import { test } from "node:test";
import { RESPONSE_TAG } from "../generated/ResponseOf.ts";

void test("the generated pairing answers each request with its response", () => {
  assert.equal(RESPONSE_TAG.ping, "pong");
  assert.equal(RESPONSE_TAG.manifest, "manifest");
  assert.equal(RESPONSE_TAG.listFiles, "files");
  assert.equal(RESPONSE_TAG.createThread, "thread");
  assert.equal(RESPONSE_TAG.clearSemanticCaches, "done");
});
