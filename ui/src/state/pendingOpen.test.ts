import assert from "node:assert/strict";
import { test } from "node:test";
import { PendingOpen } from "./pendingOpen.ts";

function setup() {
  const busy = { on: false };
  const open = new PendingOpen((on) => (busy.on = on));
  return { open, busy };
}

void test("the changeset on view is not shown from the request on, and the announcement hands over to the fetch", () => {
  const { open, busy } = setup();
  const token = open.begin();
  assert.equal(busy.on, true);
  open.announced();
  assert.equal(busy.on, false);
  assert.equal(open.settle(token), false);
});

void test("an answer without an announcement gives the changeset on view back to be fetched", () => {
  const { open, busy } = setup();
  const token = open.begin();
  assert.equal(open.settle(token), true);
  assert.equal(busy.on, false);
});

void test("the answer to an older request does not end a newer one", () => {
  const { open, busy } = setup();
  const first = open.begin();
  const second = open.begin();
  assert.equal(open.settle(first), false);
  assert.equal(busy.on, true);
  assert.equal(open.settle(second), true);
  assert.equal(busy.on, false);
});
