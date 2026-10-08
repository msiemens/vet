import assert from "node:assert/strict";
import { test } from "node:test";
import { isAbort } from "../lib/abort.ts";
import { Pending } from "./pending.ts";

const noop = () => undefined;

void test("an aborted request tells the backend to cancel it and rejects as an abort", async () => {
  const pending = new Pending<string>();
  const cancelled: number[] = [];
  const abort = new AbortController();
  const answer = pending.wait(4, abort.signal, (id) => cancelled.push(id));

  abort.abort();

  await assert.rejects(answer, isAbort);
  assert.deepEqual(cancelled, [4]);
});

void test("a request that was answered is not cancelled by a later abort", async () => {
  const pending = new Pending<string>();
  const cancelled: number[] = [];
  const abort = new AbortController();
  const answer = pending.wait(4, abort.signal, (id) => cancelled.push(id));

  pending.resolve(4, "done");
  abort.abort();

  assert.equal(await answer, "done");
  assert.deepEqual(cancelled, []);
});

void test("a signal that has fired already rejects without anything to cancel", async () => {
  const pending = new Pending<string>();
  const cancelled: number[] = [];
  const answer = pending.wait(4, AbortSignal.abort(), (id) => cancelled.push(id));

  await assert.rejects(answer, isAbort);
  assert.deepEqual(cancelled, []);
});

void test("the backend confirming a cancellation rejects the request as an abort, not as a failure", async () => {
  const pending = new Pending<string>();
  const answer = pending.wait(4, undefined, noop);

  pending.reject(4, new DOMException("cancelled", "AbortError"));

  await assert.rejects(answer, isAbort);
});

void test("a closed connection fails what waits, with its error", async () => {
  const pending = new Pending<string>();
  const answers = [pending.wait(1, undefined, noop), pending.wait(2, new AbortController().signal, noop)];

  pending.rejectAll(new Error("connection closed"));

  for (const answer of answers) await assert.rejects(answer, /connection closed/);
});
