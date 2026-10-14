import assert from "node:assert/strict";
import { test } from "node:test";
import { Latest, Resource } from "./resource.ts";

function setup() {
  const shown: string[] = [];
  const failures: unknown[] = [];
  const busy = { on: false };
  const resource = new Resource<string>({
    busy: (on) => (busy.on = on),
    apply: (value) => shown.push(value),
  });
  const handled = new Resource<string>({
    apply: (value) => shown.push(value),
    failed: (error) => failures.push(error),
  });
  return { shown, failures, busy, resource, handled };
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((res, rej) => ((resolve = res), (reject = rej)));
  return { promise, resolve, reject };
}

void test("a read is busy until its result is applied", async () => {
  const { shown, busy, resource } = setup();
  const a = deferred<string>();
  const done = resource.load(() => a.promise);
  assert.equal(busy.on, true);
  a.resolve("a");
  assert.equal(await done, "a");
  assert.deepEqual(shown, ["a"]);
  assert.equal(busy.on, false);
});

void test("an older read that settles after a newer one is not applied and settles as the newer", async () => {
  const { shown, busy, resource } = setup();
  const old = deferred<string>();
  const next = deferred<string>();
  const first = resource.load(() => old.promise);
  const second = resource.load(() => next.promise);
  next.resolve("new");
  assert.equal(await second, "new");
  old.resolve("old");
  assert.equal(await first, "new");
  assert.deepEqual(shown, ["new"]);
  assert.equal(busy.on, false);
});

void test("an older read that settles first does not end the newer one's busy state", async () => {
  const { shown, busy, resource } = setup();
  const old = deferred<string>();
  const next = deferred<string>();
  const first = resource.load(() => old.promise);
  const second = resource.load(() => next.promise);
  old.resolve("old");
  await Promise.resolve();
  assert.equal(busy.on, true);
  next.resolve("new");
  assert.equal(await first, "new");
  assert.equal(await second, "new");
  assert.deepEqual(shown, ["new"]);
});

void test("a read underway when the store is reset is dropped, with its error", async () => {
  const { shown, failures, busy, resource, handled } = setup();
  const lateResult = deferred<string>();
  const lateError = deferred<string>();
  const result = resource.load(() => lateResult.promise);
  const failing = handled.load(() => lateError.promise);
  resource.cancel();
  handled.cancel();
  assert.equal(busy.on, false);
  lateResult.resolve("old project");
  lateError.reject(new Error("closed"));
  assert.equal(await result, undefined);
  assert.equal(await failing, undefined);
  assert.deepEqual(shown, []);
  assert.deepEqual(failures, []);
  assert.equal(busy.on, false);
});

void test("an error of the latest read ends the busy state and reaches the caller", async () => {
  const { shown, busy, resource } = setup();
  const a = deferred<string>();
  const done = resource.load(() => a.promise);
  a.reject(new Error("no such changeset"));
  await assert.rejects(done, /no such changeset/);
  assert.equal(busy.on, false);
  assert.deepEqual(shown, []);
});

void test("an error of the latest read goes to the store that handles it, and an older read's does not", async () => {
  const { shown, failures, handled } = setup();
  const old = deferred<string>();
  const next = deferred<string>();
  const first = handled.load(() => old.promise);
  const second = handled.load(() => next.promise);
  old.reject(new Error("stale"));
  next.reject(new Error("gone"));
  assert.equal(await first, undefined);
  assert.equal(await second, undefined);
  assert.deepEqual(
    failures.map((e) => (e as Error).message),
    ["gone"],
  );
  assert.deepEqual(shown, []);
});

void test("an error of an older read is not reported once a newer one is underway", async () => {
  const { shown, resource } = setup();
  const old = deferred<string>();
  const next = deferred<string>();
  const first = resource.load(() => old.promise);
  const second = resource.load(() => next.promise);
  old.reject(new Error("stale"));
  next.resolve("new");
  assert.equal(await first, "new");
  assert.equal(await second, "new");
  assert.deepEqual(shown, ["new"]);
});

void test("only the read begun last is the latest, and a cancel ends them all", () => {
  const reads = new Latest();
  const first = reads.begin();
  const second = reads.begin();
  assert.deepEqual([first(), second()], [false, true]);
  reads.cancel();
  assert.equal(second(), false);
});

void test("reads that run side by side all count until the epoch they belong to moves on", () => {
  const epoch = new Latest();
  const a = epoch.mark();
  const b = epoch.mark();
  assert.deepEqual([epoch.movedSince(a), epoch.movedSince(b)], [false, false]);
  epoch.cancel();
  assert.deepEqual([epoch.movedSince(a), epoch.movedSince(b)], [true, true]);
  assert.equal(epoch.movedSince(epoch.mark()), false);
});
