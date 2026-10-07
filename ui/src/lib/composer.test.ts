import assert from "node:assert/strict";
import { test } from "node:test";
import { canSubmit, isSubmitKey } from "./composer.ts";

void test("blank text and a running request cannot be posted", () => {
  assert.equal(canSubmit("hello", false), true);
  assert.equal(canSubmit(" \n\t", false), false);
  assert.equal(canSubmit("hello", true), false);
});

void test("Enter posts only with Ctrl or Cmd", () => {
  assert.equal(isSubmitKey({ key: "Enter", ctrlKey: true, metaKey: false }), true);
  assert.equal(isSubmitKey({ key: "Enter", ctrlKey: false, metaKey: true }), true);
  assert.equal(isSubmitKey({ key: "Enter", ctrlKey: false, metaKey: false }), false);
  assert.equal(isSubmitKey({ key: "a", ctrlKey: true, metaKey: false }), false);
});
