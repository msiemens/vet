import assert from "node:assert/strict";
import { test } from "node:test";
import { type ShownSources, followShown } from "./whileShown.ts";

/** The stores' screen wiring as a test drives it. */
function setup(shown: boolean) {
  const log: string[] = [];
  let onScreen = shown;
  const listeners = {
    shown: new Set<() => void>(),
    hidden: new Set<() => void>(),
  };
  const sources: ShownSources = {
    shown: () => onScreen,
    onShown: (l) => (listeners.shown.add(l), () => listeners.shown.delete(l)),
    onHidden: (l) => (listeners.hidden.add(l), () => listeners.hidden.delete(l)),
  };
  const fire = {
    shown: () => {
      onScreen = true;
      listeners.shown.forEach((l) => l());
    },
    hidden: () => {
      onScreen = false;
      listeners.hidden.forEach((l) => l());
    },
  };
  /** How many listeners are left on the screen. */
  const listening = () => listeners.shown.size + listeners.hidden.size;
  return { log, sources, fire, listening };
}

const start = (log: string[]) => () => {
  log.push("start");
  return () => log.push("stop");
};

void test("nothing is followed until the screen is shown", () => {
  const { log, sources, fire } = setup(false);
  followShown(start(log), sources);
  assert.deepEqual(log, []);
  fire.shown();
  assert.deepEqual(log, ["start"]);
});

void test("the screen going away stops the following; coming back starts it anew", () => {
  const { log, sources, fire } = setup(true);
  followShown(start(log), sources);
  assert.deepEqual(log, ["start"]);
  fire.hidden();
  assert.deepEqual(log, ["start", "stop"]);
  fire.shown();
  assert.deepEqual(log, ["start", "stop", "start"]);
});

void test("a showing that arrives twice starts once", () => {
  const { log, sources, fire } = setup(false);
  followShown(start(log), sources);
  fire.shown();
  fire.shown();
  assert.deepEqual(log, ["start"]);
});

void test("a screen already shown follows at once", () => {
  const { log, sources } = setup(true);
  followShown(start(log), sources);
  assert.deepEqual(log, ["start"]);
});

void test("ending the following stops what runs and leaves no listener on the screen", () => {
  const { log, sources, fire, listening } = setup(true);
  const end = followShown(start(log), sources);
  assert.equal(listening(), 2);
  end();
  assert.deepEqual(log, ["start", "stop"]);
  assert.equal(listening(), 0);
  fire.shown();
  assert.deepEqual(log, ["start", "stop"], "the screen coming up starts nothing");
});

void test("the screen going away leaves its listeners to come back", () => {
  const { log, sources, fire, listening } = setup(true);
  followShown(start(log), sources);
  fire.hidden();
  assert.equal(listening(), 2, "the following is not over");
  fire.shown();
  assert.deepEqual(log, ["start", "stop", "start"]);
});
