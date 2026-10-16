import assert from "node:assert/strict";
import { test } from "node:test";
import type { SessionEvent } from "./backend.ts";
import type { Opening } from "./openedProtocol.ts";
import { type Followed, type Upstream, keepCurrent } from "./keepCurrent.ts";

function setup() {
  const log: string[] = [];
  const listeners = {
    event: new Set<(event: SessionEvent) => void>(),
    resync: new Set<() => void>(),
    opening: new Set<(opening: Opening) => void>(),
  };
  const up: Upstream = {
    onEvent: (l) => (listeners.event.add(l), () => listeners.event.delete(l)),
    onResync: (l) => (listeners.resync.add(l), () => listeners.resync.delete(l)),
    onOpening: (l) => (listeners.opening.add(l), () => listeners.opening.delete(l)),
  };
  const held: Followed = {
    reloadOn: ["threadsChanged", "snapshotSettled"],
    reload: () => log.push("reload"),
    cancel: () => log.push("cancel"),
    clear: () => log.push("clear"),
  };
  const fire = {
    event: (type: SessionEvent["type"]) => listeners.event.forEach((l) => l({ type, project: 1 } as SessionEvent)),
    resync: () => listeners.resync.forEach((l) => l()),
    opening: (opening: Opening) => listeners.opening.forEach((l) => l(opening)),
  };
  return { log, listeners, up, held, fire };
}

void test("the data is read at the start and after the events that change it, not after others", () => {
  const { log, up, held, fire } = setup();
  keepCurrent(up, held);
  assert.deepEqual(log, ["reload"]);
  fire.event("serversChanged");
  assert.deepEqual(log, ["reload"]);
  fire.event("threadsChanged");
  fire.event("snapshotSettled");
  assert.deepEqual(log, ["reload", "reload", "reload"]);
});

void test("the data is read again when the connection is back", () => {
  const { log, up, held, fire } = setup();
  keepCurrent(up, held);
  fire.resync();
  assert.deepEqual(log, ["reload", "reload"]);
});

void test("a changeset asked for drops what is held and reads nothing until it is announced", () => {
  const { log, up, held, fire } = setup();
  keepCurrent(up, held);
  log.length = 0;
  fire.opening({ type: "requested" });
  assert.deepEqual(log, ["cancel", "clear"]);
  log.length = 0;
  fire.opening({ type: "opened", changeset: { type: "workingTree" } });
  assert.deepEqual(log, ["cancel", "clear", "reload"]);
});

void test("an ask that came to nothing keeps what is held and reads it again", () => {
  const { log, up, held, fire } = setup();
  keepCurrent(up, held);
  log.length = 0;
  fire.opening({ type: "unannounced" });
  assert.deepEqual(log, ["cancel", "reload"]);
});

void test("the store's own part of an event or an opening comes first", () => {
  const { log, up, held, fire } = setup();
  keepCurrent(up, {
    ...held,
    event: (event) => log.push(`event ${event.type}`),
    opening: (opening) => log.push(`opening ${opening.type}`),
  });
  log.length = 0;
  fire.event("threadsChanged");
  fire.opening({ type: "requested" });
  assert.deepEqual(log, ["event threadsChanged", "reload", "opening requested", "cancel", "clear"]);
});

void test("stopping follows nothing more", () => {
  const { log, listeners, up, held, fire } = setup();
  const stop = keepCurrent(up, held);
  stop();
  log.length = 0;
  fire.event("threadsChanged");
  fire.resync();
  fire.opening({ type: "unannounced" });
  assert.deepEqual(log, []);
  assert.deepEqual([listeners.event.size, listeners.resync.size, listeners.opening.size], [0, 0, 0]);
});
