import assert from "node:assert/strict";
import { test } from "node:test";
import { PAGE, listKey, moveHighlight, plainKey } from "./listKeys.ts";

const key = (k: string, mods: { shiftKey?: boolean } = {}) => ({
  key: k,
  ctrlKey: false,
  altKey: false,
  shiftKey: false,
  metaKey: false,
  ...mods,
});

void test("the highlight starts at the end the key comes from and stops at both ends", () => {
  assert.equal(moveHighlight(-1, "down", 3), 0);
  assert.equal(moveHighlight(-1, "up", 3), 2);
  assert.equal(moveHighlight(2, "down", 3), 2);
  assert.equal(moveHighlight(0, "up", 3), 0);
  assert.equal(moveHighlight(1, "down", 3), 2);
});

void test("an empty list has no highlight", () => {
  assert.equal(moveHighlight(-1, "down", 0), -1);
  assert.equal(moveHighlight(4, "up", 0), -1);
  assert.equal(moveHighlight(4, "last", 0), -1);
});

void test("the highlight of a list that shrank comes back into it", () => {
  assert.equal(moveHighlight(9, "up", 3), 2);
  assert.equal(moveHighlight(9, "down", 3), 2);
});

void test("the Page keys move a page and stop at the ends", () => {
  assert.equal(moveHighlight(1, "pageDown", 100), 1 + PAGE);
  assert.equal(moveHighlight(50, "pageUp", 100), 50 - PAGE);
  assert.equal(moveHighlight(95, "pageDown", 100), 99);
  assert.equal(moveHighlight(3, "pageUp", 100), 0);
  assert.equal(moveHighlight(-1, "pageDown", 100), 0);
  assert.equal(moveHighlight(-1, "pageUp", 100), 99);
});

void test("Home and End go to the first and last item", () => {
  assert.equal(moveHighlight(5, "first", 10), 0);
  assert.equal(moveHighlight(-1, "last", 10), 9);
});

void test("the keys of a list are the arrows, the Page keys, Home, End, Space and Enter", () => {
  assert.equal(listKey(key("ArrowUp")), "up");
  assert.equal(listKey(key("PageDown")), "pageDown");
  assert.equal(listKey(key("PageUp")), "pageUp");
  assert.equal(listKey(key("Home")), "first");
  assert.equal(listKey(key("End")), "last");
  assert.equal(listKey(key(" ")), "single");
  assert.equal(listKey(key("Enter")), "double");
  assert.equal(listKey(key("a")), undefined);
  assert.equal(listKey(key("ArrowDown", { shiftKey: true })), undefined);
});

void test("beside a text field Space, Home and End are the field's and the rest is the list's", () => {
  assert.equal(listKey(key(" "), true), undefined);
  assert.equal(listKey(key("Home"), true), undefined);
  assert.equal(listKey(key("End"), true), undefined);
  assert.equal(listKey(key("PageDown"), true), "pageDown");
  assert.equal(listKey(key("ArrowDown"), true), "down");
  assert.equal(listKey(key("Enter"), true), "double");
});

void test("a plain key has no modifier", () => {
  assert.equal(plainKey(key("a")), true);
  assert.equal(plainKey(key("a", { shiftKey: true })), false);
});
