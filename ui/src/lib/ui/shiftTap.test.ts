import assert from "node:assert/strict";
import { test } from "node:test";
import { ShiftTap } from "./shiftTap.ts";

const down = (
  key: string,
  init: { repeat?: boolean; ctrlKey?: boolean; altKey?: boolean; metaKey?: boolean } = {},
) => ({
  key,
  repeat: false,
  ctrlKey: false,
  altKey: false,
  metaKey: false,
  ...init,
});

/** One tap of Shift, released at `at`. */
function tap(shift: ShiftTap, at: number): boolean {
  shift.keydown(down("Shift"));
  return shift.keyup({ key: "Shift" }, at);
}

void test("two quick taps are a double tap, the first alone is not", () => {
  const shift = new ShiftTap();
  assert.equal(tap(shift, 1000), false);
  assert.equal(tap(shift, 1200), true);
});

void test("a double tap is used up: the next tap starts a new one", () => {
  const shift = new ShiftTap();
  tap(shift, 1000);
  tap(shift, 1100);
  assert.equal(tap(shift, 1200), false);
});

void test("two taps too far apart are two single taps", () => {
  const shift = new ShiftTap();
  tap(shift, 1000);
  assert.equal(tap(shift, 1600), false);
  assert.equal(tap(shift, 1700), true, "the late tap counts as a first one");
});

void test("typing a capital letter between the taps breaks the double tap", () => {
  const shift = new ShiftTap();
  tap(shift, 1000);
  shift.keydown(down("Shift"));
  shift.keydown(down("A"));
  assert.equal(shift.keyup({ key: "Shift" }, 1100), false);
});

void test("a key typed between two taps breaks the double tap", () => {
  const shift = new ShiftTap();
  tap(shift, 1000);
  shift.keydown(down("a"));
  assert.equal(tap(shift, 1100), false);
});

void test("Shift held with Ctrl, Alt or Cmd is a modifier, not a tap", () => {
  for (const init of [{ ctrlKey: true }, { altKey: true }, { metaKey: true }]) {
    const shift = new ShiftTap();
    tap(shift, 1000);
    shift.keydown(down("Shift", init));
    assert.equal(shift.keyup({ key: "Shift" }, 1100), false, JSON.stringify(init));
  }
});

void test("holding Shift down counts once", () => {
  const shift = new ShiftTap();
  tap(shift, 1000);
  shift.keydown(down("Shift"));
  shift.keydown(down("Shift", { repeat: true }));
  assert.equal(shift.keyup({ key: "Shift" }, 1100), true);
});
