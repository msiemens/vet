import assert from "node:assert/strict";
import { test } from "node:test";
import { Lifetime } from "./lifetime.ts";
import { ReadFlush, type ReadActions } from "./readFlush.ts";
import { NO_READS, type Reads } from "./sessionReads.ts";

const tick = () => new Promise<void>((resolve) => setTimeout(resolve, 0));

/** Actions that record when they start and finish; a read named in `held` waits until `release` is called for it. */
function recorder(held: string[] = [], fed = false) {
  const log: string[] = [];
  const releases = new Map<string, () => void>();
  const read = (name: string) => async () => {
    log.push(`start ${name}`);
    if (held.includes(name)) await new Promise<void>((resolve) => releases.set(name, resolve));
    log.push(`end ${name}`);
  };
  const actions: ReadActions = {
    manifest: async () => {
      await read("manifest")();
      return fed;
    },
    whole: read("whole"),
    threads: read("threads"),
    pullRequest: read("pullRequest"),
    versions: read("versions"),
    changesSince: () => void read("changesSince")(),
    commits: read("commits"),
    scopes: () => void log.push("scopes"),
  };
  return { actions, log, release: (name: string) => releases.get(name)?.() };
}

const everything: Reads = {
  manifest: true,
  whole: { revision: 3 },
  threads: true,
  pullRequest: false,
  versions: true,
  changesSince: true,
  commits: true,
  scopes: true,
};

async function settled(flush: ReadFlush) {
  while (flush.pending) await tick();
}

void test("the reads run on the next task, not when asked for", async () => {
  const { actions, log } = recorder();
  const flush = new ReadFlush(actions, new Lifetime());

  flush.want(everything);

  assert.deepEqual(log, []);
  assert.equal(flush.pending, true);
  await settled(flush);
  assert.notDeepEqual(log, []);
});

void test("the manifest comes first, the whole changeset second, the rest together, the scopes last", async () => {
  const { actions, log } = recorder(["threads"]);
  const flush = new ReadFlush(actions, new Lifetime());

  flush.want(everything);
  await tick();
  await tick();

  // Threads is slow; the others have started beside it and the scopes wait for all of them.
  assert.deepEqual(log.slice(0, 4), ["start manifest", "end manifest", "start whole", "end whole"]);
  assert.deepEqual(
    log.slice(4).filter((entry) => entry.startsWith("start")),
    ["start threads", "start versions", "start changesSince", "start commits"],
  );
  assert.equal(log.includes("scopes"), false);
});

void test("the scopes read after the rest has finished", async () => {
  const { actions, log, release } = recorder(["threads"]);
  const flush = new ReadFlush(actions, new Lifetime());

  flush.want(everything);
  await tick();
  await tick();
  release("threads");
  await settled(flush);

  assert.equal(log.at(-1), "scopes");
  assert.ok(log.indexOf("end threads") < log.indexOf("scopes"));
});

void test("the whole changeset is not read when the manifest fed it", async () => {
  const { actions, log } = recorder([], true);
  const flush = new ReadFlush(actions, new Lifetime());

  flush.want(everything);
  await settled(flush);

  assert.equal(log.includes("start whole"), false);
  assert.equal(log.includes("start threads"), true);
});

void test("wants made before the reads start are one pass", async () => {
  const { actions, log } = recorder();
  const flush = new ReadFlush(actions, new Lifetime());

  flush.want({ ...NO_READS, threads: true });
  flush.want({ ...NO_READS, threads: true, versions: true });
  flush.want({ ...NO_READS, manifest: true });
  await settled(flush);

  assert.equal(log.filter((entry) => entry === "start threads").length, 1);
  assert.equal(log.filter((entry) => entry === "start manifest").length, 1);
  assert.equal(log.filter((entry) => entry === "start versions").length, 1);
});

void test("a want for a read underway is held for after it, not started beside it", async () => {
  const { actions, log, release } = recorder(["manifest"]);
  const flush = new ReadFlush(actions, new Lifetime());

  flush.want({ ...NO_READS, manifest: true });
  await tick();
  flush.want({ ...NO_READS, manifest: true, threads: true });
  await tick();

  assert.deepEqual(log, ["start manifest"]);
  release("manifest");
  await tick();
  await tick();
  release("manifest");
  await settled(flush);

  assert.deepEqual(log, [
    "start manifest",
    "end manifest",
    "start manifest",
    "end manifest",
    "start threads",
    "end threads",
  ]);
});

void test("disposing the lifetime cancels the reads waiting", async () => {
  const { actions, log } = recorder();
  const lifetime = new Lifetime();
  const flush = new ReadFlush(actions, lifetime);

  flush.want(everything);
  lifetime.dispose();
  await tick();

  assert.deepEqual(log, []);
  assert.equal(flush.pending, false);
});

void test("a dead lifetime starts nothing, and a pass underway stops at the next step", async () => {
  const { actions, log, release } = recorder(["manifest"]);
  const lifetime = new Lifetime();
  const flush = new ReadFlush(actions, lifetime);

  flush.want(everything);
  await tick();
  lifetime.dispose();
  release("manifest");
  await settled(flush);
  flush.want(everything);
  await tick();

  assert.deepEqual(log, ["start manifest", "end manifest"]);
  assert.equal(flush.pending, false);
});
