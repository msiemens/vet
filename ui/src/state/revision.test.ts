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

// A16: the answers that gained a revision (a base diff, an item blob, a mapping, a comment
// target, a thread) are dropped by their store through the same helper when the view moved on.
void test("a reply of another revision is dropped as a thread store would drop it", () => {
  const thread = { revision: 7, id: 1, targetLine: 4 };
  assert.equal(atRevision(7, thread), thread);
  assert.equal(atRevision(8, thread), undefined);
});
