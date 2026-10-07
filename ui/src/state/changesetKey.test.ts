import assert from "node:assert/strict";
import { test } from "node:test";
import type { ChangesetKey } from "../generated/ChangesetKey";
import { keyId, parseKeyId } from "./changesetKey.ts";

void test("a key survives being written as a command argument, whatever the branch is called", () => {
  const keys: ChangesetKey[] = [
    { type: "workingTree" },
    { type: "branch", name: "feature/login" },
    { type: "branch", name: "odd:name" },
    { type: "branch", name: "workingTree" },
  ];
  for (const key of keys) assert.deepEqual(parseKeyId(keyId(key)), key);
});

void test("an argument that names no changeset is refused", () => {
  assert.equal(parseKeyId(undefined), undefined);
  assert.equal(parseKeyId("branch:"), undefined);
  assert.equal(parseKeyId("tag:v1"), undefined);
});
