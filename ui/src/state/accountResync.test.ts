import assert from "node:assert/strict";
import { test } from "node:test";
import { AccountResync } from "./accountResync.ts";

function deferred() {
  let resolve!: () => void;
  const promise = new Promise<void>((res) => (resolve = res));
  return { promise, resolve };
}

void test("a change of accounts syncs, then reloads what the platform said", async () => {
  const log: string[] = [];
  const resync = new AccountResync({
    sync: () => (log.push("sync"), Promise.resolve()),
    reload: () => (log.push("reload"), Promise.resolve()),
  });

  await resync.changed();

  assert.deepEqual(log, ["sync", "reload"]);
});

void test("changes arriving during a run make exactly one more run, never a parallel one", async () => {
  const log: string[] = [];
  const gate = deferred();
  let first = true;
  const resync = new AccountResync({
    sync: async () => {
      log.push("sync");
      if (first) {
        first = false;
        await gate.promise;
      }
    },
    reload: () => (log.push("reload"), Promise.resolve()),
  });

  const run = resync.changed();
  await resync.changed();
  await resync.changed();
  assert.deepEqual(log, ["sync"]);
  gate.resolve();
  await run;

  assert.deepEqual(log, ["sync", "reload", "sync", "reload"]);
});

void test("a run that failed does not stop the next change from syncing", async () => {
  let syncs = 0;
  const resync = new AccountResync({
    sync: () => (++syncs === 1 ? Promise.reject(new Error("offline")) : Promise.resolve()),
    reload: () => Promise.resolve(),
  });

  await assert.rejects(resync.changed(), /offline/);
  await resync.changed();

  assert.equal(syncs, 2);
});
