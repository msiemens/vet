import assert from "node:assert/strict";
import { afterEach, beforeEach, test } from "node:test";
import { watchDoubleShift } from "./doubleShift.ts";

type Listener = (event: unknown) => void;

/** The window's events, sent by hand. */
let listeners: Map<string, Set<Listener>>;
let modalOpen: boolean;

const send = (type: string, event: object = {}) => {
  for (const listener of listeners.get(type) ?? []) listener(event);
};

const key = (type: "keydown" | "keyup", at = 0) => send(type, { key: "Shift", timeStamp: at });
/** A tap of Shift that comes up at `at`. */
const tap = (at: number) => {
  key("keydown", at - 20);
  key("keyup", at);
};

beforeEach(() => {
  listeners = new Map();
  modalOpen = false;
  Object.assign(globalThis, {
    window: {
      addEventListener: (type: string, listener: Listener) => {
        listeners.set(type, (listeners.get(type) ?? new Set()).add(listener));
      },
      removeEventListener: (type: string, listener: Listener) => void listeners.get(type)?.delete(listener),
    },
    document: { querySelector: () => (modalOpen ? {} : null) },
  });
});

afterEach(() => {
  Reflect.deleteProperty(globalThis, "window");
  Reflect.deleteProperty(globalThis, "document");
});

function watched(enabled = () => true) {
  const fired = { count: 0 };
  const stop = watchDoubleShift(enabled, () => fired.count++);
  return { fired, stop };
}

void test("two taps of Shift fire once, and not after the watch is stopped", () => {
  const { fired, stop } = watched();
  tap(1000);
  tap(1150);
  assert.equal(fired.count, 1);
  stop();
  tap(2000);
  tap(2150);
  assert.equal(fired.count, 1);
});

void test("a click, a turn of the wheel or the window losing focus between the taps break the double tap", () => {
  for (const between of ["pointerdown", "wheel", "blur"]) {
    const { fired, stop } = watched();
    tap(1000);
    send(between);
    tap(1150);
    assert.equal(fired.count, 0, between);
    stop();
  }
});

void test("it does not fire while a modal dialog is open, nor where it is not enabled", () => {
  const { fired, stop } = watched(() => false);
  tap(1000);
  tap(1150);
  assert.equal(fired.count, 0);
  stop();

  const modal = watched();
  modalOpen = true;
  tap(1000);
  tap(1150);
  assert.equal(modal.fired.count, 0);
  modalOpen = false;
  tap(3000);
  tap(3150);
  assert.equal(modal.fired.count, 1);
});
