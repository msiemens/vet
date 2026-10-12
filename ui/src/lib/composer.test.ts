import assert from "node:assert/strict";
import { test } from "node:test";
import { canSubmit, isSubmitKey, replyShape } from "./composer.ts";

void test("blank text and a running request cannot be posted", () => {
  assert.equal(canSubmit("hello", false), true);
  assert.equal(canSubmit(" \n\t", false), false);
  assert.equal(canSubmit("hello", true), false);
});

void test("a reply field stays one row, with no button, until it is used", () => {
  assert.deepEqual(replyShape("", false), { rows: 1, send: false });
  assert.deepEqual(replyShape("", true), { rows: 3, send: false });
  assert.deepEqual(replyShape("   ", true), { rows: 3, send: false });
});

void test("a reply with text keeps its room and its button when the field loses the focus", () => {
  assert.deepEqual(replyShape("later", false), { rows: 3, send: true });
});

void test("Enter posts only with Ctrl or Cmd", () => {
  assert.equal(isSubmitKey({ key: "Enter", ctrlKey: true, metaKey: false }), true);
  assert.equal(isSubmitKey({ key: "Enter", ctrlKey: false, metaKey: true }), true);
  assert.equal(isSubmitKey({ key: "Enter", ctrlKey: false, metaKey: false }), false);
  assert.equal(isSubmitKey({ key: "a", ctrlKey: true, metaKey: false }), false);
});
