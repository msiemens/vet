import assert from "node:assert/strict";
import { test } from "node:test";
import { EscapeGate } from "./escapeGate.ts";

void test("Escape with the menu open closes the menu only, whether or not the browser cancels the dialog too", () => {
  for (const cancelled of [false, true]) {
    const gate = new EscapeGate();
    assert.equal(gate.press(true), true);
    if (cancelled) assert.equal(gate.cancel(), false);
    gate.settle();
    // The next cancel is a new press's.
    assert.equal(gate.cancel(), true);
  }
});

void test("Escape with the menu shut cancels the dialog", () => {
  const gate = new EscapeGate();
  assert.equal(gate.press(false), false);
  assert.equal(gate.cancel(), true);
});
