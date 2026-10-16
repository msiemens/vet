import assert from "node:assert/strict";
import { test } from "node:test";
import type { Backend } from "./backend.ts";
import { type ShownSources, followShown } from "./whileShown.ts";

/** The stores' screen wiring as a test drives it; the backend is present once a project is open. */
function setup(shown: boolean, open: boolean) {
  const log: string[] = [];
  let onScreen = shown;
  const backend = {} as Backend;
  const listeners = {
    open: new Set<() => void>(),
    close: new Set<() => void>(),
    shown: new Set<() => void>(),
    hidden: new Set<() => void>(),
  };
  const sources: ShownSources = {
    backend: () => (open ? backend : undefined),
    shown: () => onScreen,
    onOpen: (l) => (listeners.open.add(l), () => listeners.open.delete(l)),
    onClose: (l) => (listeners.close.add(l), () => listeners.close.delete(l)),
    onShown: (l) => (listeners.shown.add(l), () => listeners.shown.delete(l)),
    onHidden: (l) => (listeners.hidden.add(l), () => listeners.hidden.delete(l)),
  };
  const fire = {
    open: () => listeners.open.forEach((l) => l()),
    close: () => listeners.close.forEach((l) => l()),
    shown: () => {
      onScreen = true;
      listeners.shown.forEach((l) => l());
    },
    hidden: () => {
      onScreen = false;
      listeners.hidden.forEach((l) => l());
    },
  };
  return { log, sources, fire };
}

const start = (log: string[]) => () => {
  log.push("start");
  return () => log.push("stop");
};

void test("a project opened while the screen is away follows nothing until it is shown", () => {
  const { log, sources, fire } = setup(false, true);
  followShown(start(log), sources);
  assert.deepEqual(log, []);
  fire.open();
  assert.deepEqual(log, [], "the screen is still away");
  fire.shown();
  assert.deepEqual(log, ["start"]);
});

void test("the screen going away stops the following; coming back starts it anew", () => {
  const { log, sources, fire } = setup(true, true);
  followShown(start(log), sources);
  assert.deepEqual(log, ["start"]);
  fire.hidden();
  assert.deepEqual(log, ["start", "stop"]);
  fire.shown();
  assert.deepEqual(log, ["start", "stop", "start"]);
});

void test("the repository closing stops what the screen showed", () => {
  const { log, sources, fire } = setup(true, true);
  followShown(start(log), sources);
  assert.deepEqual(log, ["start"]);
  fire.close();
  assert.deepEqual(log, ["start", "stop"]);
  fire.open();
  assert.deepEqual(log, ["start", "stop", "start"], "the screen is still up");
});

void test("an opening and a showing that both arrive start once", () => {
  const { log, sources, fire } = setup(false, true);
  followShown(start(log), sources);
  fire.open();
  fire.shown();
  fire.shown();
  assert.deepEqual(log, ["start"]);
});

void test("a screen already shown over an open project follows at once", () => {
  const { log, sources } = setup(true, true);
  followShown(start(log), sources);
  assert.deepEqual(log, ["start"]);
});

void test("nothing starts without a project", () => {
  const { log, sources, fire } = setup(false, false);
  followShown(start(log), sources);
  fire.shown();
  assert.deepEqual(log, []);
});
