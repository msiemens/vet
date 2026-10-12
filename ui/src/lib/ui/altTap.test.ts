import assert from "node:assert/strict";
import { test } from "node:test";
import { AltTap } from "./altTap.ts";

const down = (
  key: string,
  init: { repeat?: boolean; ctrlKey?: boolean; shiftKey?: boolean; metaKey?: boolean } = {},
) => ({
  key,
  repeat: false,
  ctrlKey: false,
  shiftKey: false,
  metaKey: false,
  ...init,
});

void test("Alt pressed and released on its own is a tap", () => {
  const alt = new AltTap();
  alt.keydown(down("Alt"));
  assert.equal(alt.keyup({ key: "Alt" }), true);
  assert.equal(alt.keyup({ key: "Alt" }), false, "a release is one tap only");
});

void test("holding Alt down does not tap again until it is released and pressed anew", () => {
  const alt = new AltTap();
  alt.keydown(down("Alt"));
  alt.keydown(down("Alt", { repeat: true }));
  assert.equal(alt.keyup({ key: "Alt" }), true);
});

void test("Alt used as a modifier is no tap, whatever comes after the key", () => {
  for (const key of ["1", "ArrowLeft", "ArrowRight", "F1", "c"]) {
    const alt = new AltTap();
    alt.keydown(down("Alt"));
    alt.keydown(down(key));
    assert.equal(alt.keyup({ key: "Alt" }), false, key);
  }
});

void test("a key released before Alt does not make it a tap either way round", () => {
  const alt = new AltTap();
  alt.keydown(down("Alt"));
  alt.keydown(down("ArrowLeft"));
  alt.keyup({ key: "ArrowLeft" });
  assert.equal(alt.keyup({ key: "Alt" }), false);
  alt.keydown(down("Alt"));
  assert.equal(alt.keyup({ key: "Alt" }), true, "a fresh press taps again");
});

void test("Alt pressed with Ctrl, Shift or Meta already down is no tap, nor is one the pointer or a lost focus interrupted", () => {
  for (const modifier of [{ ctrlKey: true }, { shiftKey: true }, { metaKey: true }]) {
    const alt = new AltTap();
    alt.keydown(down("Alt", modifier));
    assert.equal(alt.keyup({ key: "Alt" }), false, JSON.stringify(modifier));
  }
  const clicked = new AltTap();
  clicked.keydown(down("Alt"));
  clicked.cancel();
  assert.equal(clicked.keyup({ key: "Alt" }), false);
});

void test("Alt+Shift, the Windows layout switch, and Alt+Shift+digit are no tap whichever modifier goes down first", () => {
  const altFirst = new AltTap();
  altFirst.keydown(down("Alt"));
  altFirst.keydown(down("Shift", { shiftKey: true }));
  assert.equal(altFirst.keyup({ key: "Shift" }), false);
  assert.equal(altFirst.keyup({ key: "Alt" }), false);

  const shiftFirst = new AltTap();
  shiftFirst.keydown(down("Shift", { shiftKey: true }));
  shiftFirst.keydown(down("Alt", { shiftKey: true }));
  assert.equal(shiftFirst.keyup({ key: "Alt" }), false);

  for (const key of ["!", '"', "§", "1"]) {
    const alt = new AltTap();
    alt.keydown(down("Alt"));
    alt.keydown(down("Shift", { shiftKey: true }));
    alt.keydown(down(key, { shiftKey: true }));
    assert.equal(alt.keyup({ key: "Alt" }), false, key);
  }
});

void test("AltGr is no Alt tap", () => {
  const alt = new AltTap();
  alt.keydown(down("Control", { ctrlKey: true }));
  alt.keydown(down("AltGraph", { ctrlKey: true }));
  assert.equal(alt.keyup({ key: "AltGraph" }), false);
  assert.equal(alt.keyup({ key: "Alt" }), false);
});
