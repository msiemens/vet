import assert from "node:assert/strict";
import { test } from "node:test";
import { LatestLoad } from "./latestLoad.ts";

function setup() {
  const shown: string[] = [];
  const busy = { on: false };
  const loads = new LatestLoad<string>({
    busy: (on) => (busy.on = on),
    apply: (value) => shown.push(value),
  });
  return { shown, busy, loads };
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((res, rej) => ((resolve = res), (reject = rej)));
  return { promise, resolve, reject };
}

void test("a load is busy until its result is applied", async () => {
  const { shown, busy, loads } = setup();
  const a = deferred<string>();
  const done = loads.load(() => a.promise);
  assert.equal(busy.on, true);
  a.resolve("a");
  assert.equal(await done, "a");
  assert.deepEqual(shown, ["a"]);
  assert.equal(busy.on, false);
});

void test("an older load that settles after a newer one is not applied and settles as the newer", async () => {
  const { shown, busy, loads } = setup();
  const old = deferred<string>();
  const next = deferred<string>();
  const first = loads.load(() => old.promise);
  const second = loads.load(() => next.promise);
  next.resolve("new");
  assert.equal(await second, "new");
  old.resolve("old");
  assert.equal(await first, "new");
  assert.deepEqual(shown, ["new"]);
  assert.equal(busy.on, false);
});

void test("an older load that settles first does not end the newer one's busy state", async () => {
  const { shown, busy, loads } = setup();
  const old = deferred<string>();
  const next = deferred<string>();
  const first = loads.load(() => old.promise);
  const second = loads.load(() => next.promise);
  old.resolve("old");
  await Promise.resolve();
  assert.equal(busy.on, true);
  next.resolve("new");
  assert.equal(await first, "new");
  assert.equal(await second, "new");
  assert.deepEqual(shown, ["new"]);
});

void test("a load underway when the store is reset is dropped, with its error", async () => {
  const { shown, busy, loads } = setup();
  const lateResult = deferred<string>();
  const lateError = deferred<string>();
  const result = loads.load(() => lateResult.promise);
  const failing = loads.load(() => lateError.promise);
  loads.cancel();
  assert.equal(busy.on, false);
  lateResult.resolve("old project");
  lateError.reject(new Error("closed"));
  assert.equal(await result, undefined);
  assert.equal(await failing, undefined);
  assert.deepEqual(shown, []);
  assert.equal(busy.on, false);
});

void test("an error of the latest load ends the busy state and reaches the caller", async () => {
  const { shown, busy, loads } = setup();
  const a = deferred<string>();
  const done = loads.load(() => a.promise);
  a.reject(new Error("no such changeset"));
  await assert.rejects(done, /no such changeset/);
  assert.equal(busy.on, false);
  assert.deepEqual(shown, []);
});

void test("an error of an older load is not reported once a newer one is underway", async () => {
  const { shown, loads } = setup();
  const old = deferred<string>();
  const next = deferred<string>();
  const first = loads.load(() => old.promise);
  const second = loads.load(() => next.promise);
  old.reject(new Error("stale"));
  next.resolve("new");
  assert.equal(await first, "new");
  assert.equal(await second, "new");
  assert.deepEqual(shown, ["new"]);
});
