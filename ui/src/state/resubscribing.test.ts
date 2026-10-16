import assert from "node:assert/strict";
import { test } from "node:test";
import { type Holder, resubscribing } from "./resubscribing.ts";

/** A thing on view that records who is listening to it. */
function thing(name: string, log: string[]) {
  return {
    name,
    attach: (listener: string) => (log.push(`on ${name}:${listener}`), () => log.push(`off ${name}:${listener}`)),
  };
}

function setup() {
  const log: string[] = [];
  let now: ReturnType<typeof thing> | undefined;
  let replaced: (next: ReturnType<typeof thing> | undefined) => void = () => undefined;
  const holder: Holder<ReturnType<typeof thing>> = {
    now: () => now,
    onReplaced: (listener) => (replaced = listener),
  };
  const subscribe = resubscribing(holder, (on, listener: string) => on.attach(listener));
  const replace = (next: ReturnType<typeof thing> | undefined) => {
    now = next;
    replaced(next);
  };
  return { log, subscribe, replace };
}

void test("a listener moves from the thing replaced to the next, and goes with none on view", () => {
  const { log, subscribe, replace } = setup();
  const a = thing("a", log);
  const b = thing("b", log);
  replace(a);
  subscribe("x");
  replace(b);
  replace(undefined);
  assert.deepEqual(log, ["on a:x", "off a:x", "on b:x", "off b:x"]);
});

void test("a listener added while nothing is on view is put on the first thing, and a stopped one is not moved", () => {
  const { log, subscribe, replace } = setup();
  const stop = subscribe("x");
  subscribe("y");
  replace(thing("a", log));
  stop();
  replace(thing("b", log));
  assert.deepEqual(log, ["on a:x", "on a:y", "off a:x", "off a:y", "on b:y"]);
});
