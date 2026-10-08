import assert from "node:assert/strict";
import { test } from "node:test";
import { StaleRequests } from "./staleRequests.ts";

void test("a newer revision aborts the requests about older ones and spares the others", () => {
  const stale = new StaleRequests();
  const old = stale.track(2);
  const older = stale.track(1);
  const current = stale.track(3);

  stale.moveTo(3);

  assert.equal(old.signal.aborted, true);
  assert.equal(older.signal.aborted, true);
  assert.equal(current.signal.aborted, false);
});

void test("the same revision again aborts nothing", () => {
  const stale = new StaleRequests();
  const request = stale.track(5);

  stale.moveTo(5);

  assert.equal(request.signal.aborted, false);
});

void test("a request that settled is no longer aborted", () => {
  const stale = new StaleRequests();
  const request = stale.track(1);
  request.release();

  stale.moveTo(2);

  assert.equal(request.signal.aborted, false);
});

void test("the caller's own signal aborts the request too, also when it fired before", () => {
  const stale = new StaleRequests();
  const caller = new AbortController();
  const request = stale.track(1, caller.signal);
  assert.equal(request.signal.aborted, false);

  caller.abort();
  assert.equal(request.signal.aborted, true);
  assert.equal(stale.track(1, caller.signal).signal.aborted, true);
});
