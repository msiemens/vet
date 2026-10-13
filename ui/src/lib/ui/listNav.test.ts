import assert from "node:assert/strict";
import { test } from "node:test";
import { ListNav } from "./listNav.ts";

const key = (k: string, mods: { ctrlKey?: boolean; metaKey?: boolean } = {}) => ({
  key: k,
  ctrlKey: false,
  altKey: false,
  shiftKey: false,
  metaKey: false,
  ...mods,
});

const wait = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

function list(items = ["a", "b", "c"]) {
  const log: string[] = [];
  let highlight = -1;
  const nav = new ListNav<string>({
    items: () => items,
    single: (s) => log.push(`single ${s}`),
    double: (s) => log.push(`double ${s}`),
    highlight: { get: () => highlight, set: (i) => (highlight = i) },
    reveal: (i) => log.push(`reveal ${i}`),
    element: () => undefined,
    ms: () => 10,
  });
  return { nav, log, highlight: () => highlight };
}

void test("Up and Down only move the highlight", () => {
  const { nav, log, highlight } = list();
  assert.equal(nav.keydown(key("ArrowDown")), true);
  assert.equal(nav.keydown(key("ArrowDown")), true);
  assert.equal(highlight(), 1);
  nav.keydown(key("ArrowUp"));
  assert.equal(highlight(), 0);
  assert.deepEqual(log, ["reveal 0", "reveal 1", "reveal 0"]);
});

void test("Space does the single action on the highlighted item and Enter the double one", () => {
  const { nav, log } = list();
  nav.keydown(key("ArrowDown"));
  nav.keydown(key("ArrowDown"));
  nav.keydown(key(" "));
  nav.keydown(key("Enter"));
  assert.deepEqual(
    log.filter((l) => !l.startsWith("reveal")),
    ["single b", "double b"],
  );
});

void test("Space and Enter with nothing highlighted do nothing", () => {
  const { nav, log } = list();
  nav.keydown(key(" "));
  nav.keydown(key("Enter"));
  assert.deepEqual(log, []);
});

void test("the Page keys, Home and End move the highlight", () => {
  const { nav, highlight } = list(Array.from({ length: 30 }, (_, i) => `i${i}`));
  nav.keydown(key("PageDown"));
  nav.keydown(key("PageDown"));
  assert.equal(highlight(), 0 + 8);
  nav.keydown(key("End"));
  assert.equal(highlight(), 29);
  nav.keydown(key("PageUp"));
  assert.equal(highlight(), 21);
  nav.keydown(key("Home"));
  assert.equal(highlight(), 0);
});

void test("beside a text field Space and Home are left to the text", () => {
  const log: string[] = [];
  let highlight = 1;
  const nav = new ListNav<string>({
    items: () => ["a", "b", "c"],
    single: (s) => log.push(`single ${s}`),
    highlight: { get: () => highlight, set: (i) => (highlight = i) },
    field: true,
  });
  assert.equal(nav.keydown(key(" ")), false);
  assert.equal(nav.keydown(key("Home")), false);
  assert.equal(nav.keydown(key("PageDown")), true);
  assert.equal(highlight, 2);
  assert.deepEqual(log, []);
});

void test("a list with one action does it at once on a click and on Enter, and once on a double click", () => {
  const log: string[] = [];
  let highlight = -1;
  const nav = new ListNav<string>({
    items: () => ["a", "b"],
    single: (s) => log.push(s),
    highlight: { get: () => highlight, set: (i) => (highlight = i) },
  });
  nav.click({ detail: 1 }, 1);
  nav.click({ detail: 2 }, 1);
  assert.deepEqual(log, ["b"]);
  nav.keydown(key("Enter"));
  assert.deepEqual(log, ["b", "b"]);
});

void test("only a pointer that moved takes the highlight", () => {
  const { nav, highlight } = list();
  nav.hover({ movementX: 0, movementY: 0 }, 2);
  assert.equal(highlight(), -1);
  nav.hover({ movementX: 0, movementY: 3 }, 2);
  assert.equal(highlight(), 2);
});

void test("a list that takes the focus starts its highlight where it says, unless it has one", () => {
  let highlight = -1;
  const nav = new ListNav<string>({
    items: () => ["a", "b", "c"],
    single: () => undefined,
    highlight: { get: () => highlight, set: (i) => (highlight = i) },
    start: () => 1,
  });
  nav.focusin();
  assert.equal(highlight, 1);
  highlight = 2;
  nav.focusin();
  assert.equal(highlight, 2);
});

void test("a key with a modifier is not the list's", () => {
  const { nav, log, highlight } = list();
  assert.equal(nav.keydown(key("ArrowDown", { metaKey: true })), false);
  assert.equal(nav.keydown(key(" ", { ctrlKey: true })), false);
  assert.equal(nav.keydown(key("a")), false);
  assert.equal(highlight(), -1);
  assert.deepEqual(log, []);
});

void test("a single click highlights at once but acts only after the double-click time, and a double click acts once", async () => {
  const { nav, log, highlight } = list();
  nav.click({ detail: 1 }, 2);
  assert.equal(highlight(), 2);
  assert.deepEqual(log, []);
  await wait(30);
  assert.deepEqual(log, ["single c"]);
  nav.click({ detail: 1 }, 0);
  nav.click({ detail: 2 }, 0);
  await wait(30);
  assert.deepEqual(log, ["single c", "double a"]);
});

void test("Enter on a row whose click waits opens it without the single action", async () => {
  const { nav, log } = list();
  nav.click({ detail: 1 }, 1);
  nav.keydown(key("Enter"));
  await wait(30);
  assert.deepEqual(log, ["double b"]);
});
