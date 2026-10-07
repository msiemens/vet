import assert from "node:assert/strict";
import { test } from "node:test";
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

void test("cleared requests are not joined", async () => {
  const flight = new InFlight<string>();
  const old = deferred<string>();
  const stale = flight.share("a", () => old.promise);
  flight.clear();

  assert.equal(await flight.share("a", () => Promise.resolve("fresh")), "fresh");
  old.resolve("stale");
  assert.equal(await stale, "stale");
});
