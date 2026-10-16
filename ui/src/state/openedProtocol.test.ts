import assert from "node:assert/strict";
import { test } from "node:test";
import type { ChangesetKey } from "../generated/ChangesetKey.ts";
import { OpenedProtocol } from "./openedProtocol.ts";

const branch = (name: string): ChangesetKey => ({ type: "branch", name });

/** A protocol whose effects write what they do into a log, in order. */
function setup() {
  const log: string[] = [];
  let busy = false;
  const reads: (() => void)[] = [];
  const protocol: OpenedProtocol = new OpenedProtocol({
    dispose: () => log.push("dispose"),
    build: (key, revision) => log.push(`build ${key.type === "branch" ? key.name : key.type}@${revision}`),
    start: (revision) => log.push(`start@${revision}`),
    notify: (opening) => log.push(`notify ${opening.type}`),
    busy: (on) => {
      busy = on;
      log.push(`busy ${on}`);
    },
    read: () => {
      log.push("read");
      return new Promise<void>((resolve) => reads.push(resolve));
    },
  });
  return { protocol, log, reads, busy: () => busy };
}

void test("a request ends the session at once, and the announcement builds the next, told before it runs", () => {
  const { protocol, log, busy } = setup();
  const token = protocol.request(branch("a"));
  assert.equal(busy(), true);
  assert.deepEqual(log, ["busy true", "dispose", "notify requested"]);
  log.length = 0;
  protocol.announced(branch("a"), 7);
  assert.equal(busy(), false);
  assert.deepEqual(log, ["busy false", "dispose", "build a@7", "notify opened", "start@7"]);
  log.length = 0;
  void protocol.answered(token);
  assert.deepEqual(log, [], "an answer to an announced request reads nothing");
});

void test("an answer without an announcement builds from the manifest", async () => {
  const { protocol, log, reads, busy } = setup();
  const token = protocol.request(branch("a"));
  log.length = 0;
  const answered = protocol.answered(token);
  assert.equal(busy(), false);
  assert.deepEqual(log, ["busy false", "notify unannounced", "read"]);
  reads[0]();
  await answered;
});

void test("a second request overtakes the first: its answer and announcement build nothing", async () => {
  const { protocol, log, busy } = setup();
  const first = protocol.request(branch("a"));
  const second = protocol.request(branch("b"));
  log.length = 0;
  await protocol.answered(first);
  assert.equal(busy(), true, "the newer request is still pending");
  protocol.announced(branch("a"), 3);
  assert.equal(busy(), true);
  assert.deepEqual(log, [], "an earlier request's changeset is not shown for a moment");
  protocol.announced(branch("b"), 4);
  assert.equal(busy(), false);
  assert.deepEqual(log, ["busy false", "dispose", "build b@4", "notify opened", "start@4"]);
  log.length = 0;
  await protocol.answered(second);
  assert.deepEqual(log, []);
});

void test("a changeset opened by another window is built when no request is pending", () => {
  const { protocol, log } = setup();
  protocol.announced(branch("x"), 2);
  assert.deepEqual(log, ["dispose", "build x@2", "notify opened", "start@2"]);
});

void test("closing while a request is out ends the session and drops the late answer and announcement", async () => {
  const { protocol, log, busy } = setup();
  const token = protocol.request(branch("a"));
  log.length = 0;
  protocol.close();
  assert.equal(busy(), false);
  assert.deepEqual(log, ["busy false", "dispose"]);
  log.length = 0;
  await protocol.answered(token);
  assert.deepEqual(log, [], "the answer after the close reads nothing");
});

void test("a read is outdated once something was asked for, announced or closed after it began", () => {
  const { protocol } = setup();
  const afterRequest = protocol.beginRead();
  protocol.request(branch("a"));
  assert.equal(protocol.readCurrent(afterRequest), false);

  const afterAnnouncement = protocol.beginRead();
  protocol.announced(branch("a"), 1);
  assert.equal(protocol.readCurrent(afterAnnouncement), false);

  const afterClose = protocol.beginRead();
  protocol.close();
  assert.equal(protocol.readCurrent(afterClose), false);

  const older = protocol.beginRead();
  const newer = protocol.beginRead();
  assert.equal(protocol.readCurrent(older), false);
  assert.equal(protocol.readCurrent(newer), true);
});
