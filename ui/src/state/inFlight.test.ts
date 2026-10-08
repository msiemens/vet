import assert from "node:assert/strict";
import { test } from "node:test";
import { isAbort } from "../lib/abort.ts";
import { InFlight } from "./inFlight.ts";

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((r) => (resolve = r));
  return { promise, resolve };
}

void test("callers asking the same question while it is underway share one request", async () => {
  const flight = new InFlight<string>();
  const request = deferred<string>();
  let started = 0;
  const start = () => (started++, request.promise);

  const first = flight.share("hover:a", start);
  const second = flight.share("hover:a", start);
  request.resolve("answer");

  assert.deepEqual(await Promise.all([first, second]), ["answer", "answer"]);
  assert.equal(started, 1);
});

void test("different questions are asked separately", async () => {
  const flight = new InFlight<string>();
  let started = 0;
  const start = () => (started++, Promise.resolve("x"));

  await Promise.all([flight.share("a", start), flight.share("b", start)]);

  assert.equal(started, 2);
});

void test("a question is asked afresh once its answer has arrived", async () => {
  const flight = new InFlight<number>();
  let started = 0;
  const start = () => Promise.resolve(++started);

  assert.equal(await flight.share("a", start), 1);
  assert.equal(await flight.share("a", start), 2);
});

void test("a failed request is not kept for the next caller", async () => {
  const flight = new InFlight<number>();
  await assert.rejects(flight.share("a", () => Promise.reject(new Error("down"))));

  assert.equal(await flight.share("a", () => Promise.resolve(7)), 7);
});

/** A promise executor that never settles. */
const never = () => undefined;

/** A request that never answers and records whether it was told to stop. */
function endless() {
  const seen = { aborted: false };
  const start = (signal: AbortSignal) => {
    signal.addEventListener("abort", () => (seen.aborted = true));
    return new Promise<string>(never);
  };
  return { seen, start };
}

void test("the request is aborted when its only caller gives up, and that caller rejects at once", async () => {
  const flight = new InFlight<string>();
  const { seen, start } = endless();
  const abort = new AbortController();
  const answer = flight.share("a", start, abort.signal);

  abort.abort();

  await assert.rejects(answer, isAbort);
  assert.equal(seen.aborted, true);
});

void test("a request another caller still wants runs on when one caller gives up", async () => {
  const flight = new InFlight<string>();
  const request = deferred<string>();
  let aborted = false;
  const start = (signal: AbortSignal) => {
    signal.addEventListener("abort", () => (aborted = true));
    return request.promise;
  };
  const leaving = new AbortController();
  const gone = flight.share("a", start, leaving.signal);
  const staying = flight.share("a", start, new AbortController().signal);

  leaving.abort();
  request.resolve("answer");

  await assert.rejects(gone, isAbort);
  assert.equal(await staying, "answer");
  assert.equal(aborted, false);
});

void test("a caller without a signal keeps the request alive", async () => {
  const flight = new InFlight<string>();
  const request = deferred<string>();
  let aborted = false;
  const start = (signal: AbortSignal) => {
    signal.addEventListener("abort", () => (aborted = true));
    return request.promise;
  };
  const leaving = new AbortController();
  const gone = flight.share("a", start, leaving.signal);
  const staying = flight.share("a", start);

  leaving.abort();
  request.resolve("answer");

  await assert.rejects(gone, isAbort);
  assert.equal(await staying, "answer");
  assert.equal(aborted, false);
});

void test("a question whose request was aborted is asked afresh", async () => {
  const flight = new InFlight<string>();
  const abort = new AbortController();
  const gone = flight.share("a", endless().start, abort.signal);
  abort.abort();
  await assert.rejects(gone, isAbort);

  assert.equal(await flight.share("a", () => Promise.resolve("fresh")), "fresh");
});

void test("cleared requests are not joined", async () => {
  const flight = new InFlight<string>();
  const old = deferred<string>();
  const stale = flight.share("a", () => old.promise);
  flight.clear();

  assert.equal(await flight.share("a", () => Promise.resolve("fresh")), "fresh");
  old.resolve("stale");
  assert.equal(await stale, "stale");
});
