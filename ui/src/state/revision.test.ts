import assert from "node:assert/strict";
import { test } from "node:test";
import { atRevision } from "./revision.ts";

void test("only a reply of the asked revision is kept", () => {
  const reply = { revision: 3, entries: [] };
  assert.equal(atRevision(3, reply), reply);
  assert.equal(atRevision(4, reply), undefined);
  assert.equal(atRevision(3, undefined), undefined);
  assert.equal(atRevision(undefined, reply), undefined);
});
