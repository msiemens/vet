import assert from "node:assert/strict";
import { beforeEach, mock, test } from "node:test";
import { GRACE_MS, HOVER_MS, HoverPopup, wordRange } from "./hoverPopup.ts";

interface Target {
  key: string;
}
const foo: Target = { key: "foo" };
const bar: Target = { key: "bar" };

/** A host that answers every query at once and records what is on screen and how often popups were built. */
function setup() {
  const built: string[] = [];
  const onScreen = new Set<string>();
  const popup = new HoverPopup<Target, string>({
    query: (t) => Promise.resolve(t.key === "empty" ? undefined : `popup of ${t.key}`),
    open: (t, content) => {
      built.push(t.key);
      onScreen.add(content);
      return () => onScreen.delete(content);
    },
  });
  return { popup, built, onScreen };
}

/** Lets the pending query's answer through. */
const answered = async () => {
  for (let i = 0; i < 5; i++) await Promise.resolve();
};

beforeEach(() => {
  mock.timers.reset();
  mock.timers.enable({ apis: ["setTimeout"] });
});

async function hoverFor(popup: HoverPopup<Target, string>, target: Target) {
  popup.pointer(target);
  mock.timers.tick(HOVER_MS);
  await answered();
}

void test("a popup is built once while the pointer stays on its symbol, however it moves", async () => {
  const { popup, built, onScreen } = setup();
  await hoverFor(popup, foo);
  for (let i = 0; i < 20; i++) {
    popup.pointer(foo);
    mock.timers.tick(HOVER_MS + GRACE_MS);
    await answered();
  }
  assert.deepEqual(built, ["foo"]);
  assert.deepEqual([...onScreen], ["popup of foo"]);
});

void test("the pointer can reach the popup within the grace period, and the popup stays while it is on it", async () => {
  const { popup, onScreen } = setup();
  await hoverFor(popup, foo);
  popup.pointer(undefined);
  mock.timers.tick(GRACE_MS - 1);
  popup.pointer(undefined, true);
  mock.timers.tick(GRACE_MS * 10);
  assert.equal(onScreen.size, 1);
});

void test("the popup closes once the pointer has been off both the symbol and the popup for the grace period", async () => {
  const { popup, onScreen } = setup();
  await hoverFor(popup, foo);
  popup.pointer(undefined);
  mock.timers.tick(GRACE_MS - 1);
  assert.equal(onScreen.size, 1);
  mock.timers.tick(1);
  assert.equal(onScreen.size, 0);
  assert.equal(popup.isOpen, false);
});

void test("leaving the popup starts the grace period again, and coming back to the symbol keeps the popup", async () => {
  const { popup, built, onScreen } = setup();
  await hoverFor(popup, foo);
  popup.pointer(undefined, true);
  popup.pointer(undefined);
  mock.timers.tick(GRACE_MS - 1);
  popup.pointer(foo);
  mock.timers.tick(GRACE_MS * 10);
  assert.equal(onScreen.size, 1);
  assert.deepEqual(built, ["foo"]);
});

void test("another symbol replaces the popup when its answer arrives, with none missing in between", async () => {
  const { popup, built, onScreen } = setup();
  await hoverFor(popup, foo);
  popup.pointer(bar);
  assert.deepEqual([...onScreen], ["popup of foo"]);
  mock.timers.tick(HOVER_MS);
  await answered();
  assert.deepEqual([...onScreen], ["popup of bar"]);
  assert.deepEqual(built, ["foo", "bar"]);
  mock.timers.tick(GRACE_MS * 10);
  assert.deepEqual([...onScreen], ["popup of bar"]);
});

void test("a symbol the pointer already left is not shown when its answer comes late", async () => {
  const { popup, built } = setup();
  popup.pointer(foo);
  mock.timers.tick(HOVER_MS);
  popup.pointer(undefined);
  await answered();
  assert.deepEqual(built, []);
});

const never = () => undefined;

/** A host whose queries never answer, and the signals they were given. */
function silent() {
  const signals: AbortSignal[] = [];
  const popup = new HoverPopup<Target, string>({
    query: (_, signal) => {
      signals.push(signal);
      return new Promise<string | undefined>(never);
    },
    open: () => never,
  });
  return { popup, signals };
}

void test("a query is aborted when the pointer moves to another symbol or off the symbol", () => {
  const { popup, signals } = silent();
  popup.pointer(foo);
  mock.timers.tick(HOVER_MS);
  popup.pointer(bar);
  assert.deepEqual(
    signals.map((s) => s.aborted),
    [true],
  );

  mock.timers.tick(HOVER_MS);
  popup.pointer(undefined);
  assert.deepEqual(
    signals.map((s) => s.aborted),
    [true, true],
  );
});

void test("a query is aborted when the popup is closed or dismissed", () => {
  const { popup, signals } = silent();
  popup.pointer(foo);
  mock.timers.tick(HOVER_MS);
  popup.dismiss();
  popup.pointer(bar);
  mock.timers.tick(HOVER_MS);
  popup.close();
  assert.deepEqual(
    signals.map((s) => s.aborted),
    [true, true],
  );
});

void test("a query the pointer stays on is left running", () => {
  const { popup, signals } = silent();
  popup.pointer(foo);
  mock.timers.tick(HOVER_MS);
  popup.pointer(foo);
  mock.timers.tick(HOVER_MS * 10);
  assert.deepEqual(
    signals.map((s) => s.aborted),
    [false],
  );
});

void test("a query that rejects as an abort leaves no popup and no error", async () => {
  const popup = new HoverPopup<Target, string>({
    query: (_, signal) =>
      new Promise((_, reject) =>
        signal.addEventListener("abort", () => reject(new DOMException("cancelled", "AbortError"))),
      ),
    open: () => assert.fail("nothing to show"),
  });
  popup.pointer(foo);
  mock.timers.tick(HOVER_MS);
  popup.pointer(undefined);
  await answered();
  assert.equal(popup.isOpen, false);
});

void test("a symbol with nothing to show leaves the earlier popup to its grace period", async () => {
  const { popup, onScreen } = setup();
  await hoverFor(popup, foo);
  await hoverFor(popup, { key: "empty" });
  assert.equal(onScreen.size, 1);
  mock.timers.tick(GRACE_MS);
  assert.equal(onScreen.size, 0);
});

void test("a dismissed popup (Escape, scroll, a change) closes at once and stays closed on its symbol until the pointer has left it", async () => {
  const { popup, built, onScreen } = setup();
  await hoverFor(popup, foo);
  popup.dismiss();
  assert.equal(onScreen.size, 0);
  await hoverFor(popup, foo);
  assert.deepEqual(built, ["foo"]);
  popup.pointer(undefined);
  await hoverFor(popup, foo);
  assert.deepEqual(built, ["foo", "foo"]);
});

void test("a word is what a hover range covers", () => {
  assert.deepEqual(wordRange("let foo_bar = x.y;", 6), { from: 4, to: 11 });
  assert.deepEqual(wordRange("let foo_bar = x.y;", 4), { from: 4, to: 11 });
  assert.deepEqual(wordRange("let foo_bar = x.y;", 15), { from: 15, to: 16 });
  assert.deepEqual(wordRange("let foo_bar = x.y;", 16), { from: 16, to: 17 });
  assert.equal(wordRange("let  x", 4), undefined);
  assert.equal(wordRange("let", 3), undefined);
});
