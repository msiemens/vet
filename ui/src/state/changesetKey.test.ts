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
    { type: "pullRequest", remote: "github.com/acme/app", number: 482 },
    { type: "pullRequest", remote: "ghe.corp.example:8443/acme/app", number: 7 },
  ];
  for (const key of keys) assert.deepEqual(parseKeyId(keyId(key)), key);
});

void test("an argument that names no changeset is refused", () => {
  assert.equal(parseKeyId(undefined), undefined);
  assert.equal(parseKeyId("branch:"), undefined);
  assert.equal(parseKeyId("tag:v1"), undefined);
  for (const id of [
    "pullRequest:",
    "pullRequest:github.com/acme/app",
    "pullRequest:github.com/acme/app#0",
    "pullRequest:github.com/acme/app#x",
    "pullRequest:#4",
  ])
    assert.equal(parseKeyId(id), undefined, id);
});

void test("two pull requests, and a pull request and a branch, never share a key", () => {
  const ids = [
    { type: "pullRequest", remote: "github.com/acme/app", number: 1 },
    { type: "pullRequest", remote: "github.com/acme/app", number: 2 },
    { type: "pullRequest", remote: "github.com/acme/lib", number: 1 },
    { type: "branch", name: "github.com/acme/app#1" },
  ].map((key) => keyId(key as ChangesetKey));
  assert.equal(new Set(ids).size, ids.length);
});
