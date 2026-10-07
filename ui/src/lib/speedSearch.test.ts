import assert from "node:assert/strict";
import { test } from "node:test";
import { bestMatch, highlightRuns, match, speedSearchKey, type SpeedSearchKeyEvent } from "./speedSearch.ts";

const ranges = (query: string, text: string) => match(query, text)?.ranges;
const score = (query: string, text: string) => match(query, text)?.score ?? -Infinity;

void test("the capitals of a name's words match it, and the match reports where", () => {
  assert.deepEqual(ranges("CSV", "ChangesetView"), [
    [0, 1],
    [6, 7],
    [9, 10],
  ]);
  assert.deepEqual(ranges("csv", "ChangesetView"), ranges("CSV", "ChangesetView"));
});

void test("words are also split at separators and acronyms", () => {
  assert.deepEqual(ranges("rc", "release_candidate"), [
    [0, 1],
    [8, 9],
  ]);
  assert.deepEqual(ranges("vc", "vet-core"), [
    [0, 1],
    [4, 5],
  ]);
  assert.deepEqual(ranges("xp", "XMLParser"), [
    [0, 1],
    [3, 4],
  ]);
});

void test("the characters of a match run on from a word's start", () => {
  assert.deepEqual(ranges("chaV", "ChangesetView"), [
    [0, 3],
    [9, 10],
  ]);
});

void test("a plain run of the text matches wherever it is", () => {
  assert.deepEqual(ranges("ange", "ChangesetView"), [[2, 6]]);
  assert.deepEqual(ranges("setV", "ChangesetView"), [[6, 10]]);
  assert.deepEqual(ranges(".git", "a.gitignore"), [[1, 5]]);
});

void test("a word cannot be skipped to reach the inside of a later one", () => {
  assert.equal(match("ci", "ChangesetView"), undefined);
  assert.equal(match("xyz", "ChangesetView"), undefined);
  assert.equal(match("longerthanthetext", "short"), undefined);
});

void test("an empty query matches every name and marks nothing", () => {
  assert.deepEqual(match("", "anything"), { score: 0, ranges: [] });
});

void test("a name that starts with the query ranks above one with the query at a later word", () => {
  assert.ok(score("ma", "main.rs") > score("ma", "Cargo_main"));
});

void test("characters on word starts rank above characters scattered within one word", () => {
  assert.ok(score("cv", "ChangesetView") > score("cv", "Canvas"));
});

void test("a match on word starts ranks above a plain run in the middle of a word", () => {
  assert.ok(score("ge", "GroupEntry") > score("ge", "Image"));
});

void test("of equal matches the shorter name ranks higher", () => {
  assert.ok(score("ab", "ab") > score("ab", "ab_longer_name"));
});

void test("the best match is the highest ranked, the first of equals, and -1 for none", () => {
  assert.equal(bestMatch("cs", ["xcs", "CacheStore", "CsvStore"]), 2);
  assert.equal(bestMatch("a", ["b", "a1", "a2"]), 1);
  assert.equal(bestMatch("q", ["b", "a"]), -1);
});

void test("highlight runs cover the text exactly", () => {
  assert.deepEqual(highlightRuns("ledger-sync", [[0, 3]]), [
    { text: "led", hit: true },
    { text: "ger-sync", hit: false },
  ]);
  assert.deepEqual(highlightRuns("abc", []), [{ text: "abc", hit: false }]);
  assert.deepEqual(
    highlightRuns("abcd", [
      [1, 2],
      [3, 4],
    ]).map((r) => r.text),
    ["a", "b", "c", "d"],
  );
});

const key = (k: string, extra: Partial<SpeedSearchKeyEvent> = {}): SpeedSearchKeyEvent => ({
  key: k,
  code: "",
  ctrlKey: false,
  altKey: false,
  shiftKey: false,
  metaKey: false,
  ...extra,
});

/** A target inside an element that matches `selector`, as `closest` answers. */
const inside = (tag: string) =>
  ({ closest: (selector: string) => (selector.includes(tag) ? {} : null) }) as EventTarget;

void test("typing characters builds the query", () => {
  assert.equal(speedSearchKey("", key("c")), "c");
  assert.equal(speedSearchKey("c", key("S", { shiftKey: true })), "cS");
  assert.equal(speedSearchKey("ab", key("-")), "ab-");
});

void test("a space joins a query but does not start one, so it can still press a button", () => {
  assert.equal(speedSearchKey("", key(" ")), undefined);
  assert.equal(speedSearchKey("a", key(" ")), "a ");
});

void test("keys typed into a text field or an editor are not the search's", () => {
  for (const tag of ["input", "textarea", "contenteditable"]) {
    assert.equal(speedSearchKey("", key("a", { target: inside(tag) })), undefined, tag);
    assert.equal(speedSearchKey("ab", key("Backspace", { target: inside(tag) })), undefined, tag);
    assert.equal(speedSearchKey("ab", key("Escape", { target: inside(tag) })), undefined, tag);
  }
});

void test("Escape clears an active search and is left to its owner (a dialog, a menu) otherwise", () => {
  assert.equal(speedSearchKey("abc", key("Escape")), "");
  assert.equal(speedSearchKey("", key("Escape")), undefined);
});

void test("Backspace removes the last character, a whole one even outside the BMP", () => {
  assert.equal(speedSearchKey("abc", key("Backspace")), "ab");
  assert.equal(speedSearchKey("a😀", key("Backspace")), "a");
  assert.equal(speedSearchKey("", key("Backspace")), undefined);
});

void test("shortcuts, navigation keys and composition are not typing", () => {
  assert.equal(speedSearchKey("a", key("b", { ctrlKey: true })), undefined);
  assert.equal(speedSearchKey("a", key("b", { metaKey: true })), undefined);
  for (const named of ["ArrowDown", "Enter", "Tab", "Home", "F4", "Shift"]) {
    assert.equal(speedSearchKey("a", key(named)), undefined, named);
  }
  assert.equal(speedSearchKey("a", key("Dead")), undefined);
  assert.equal(speedSearchKey("a", key("x", { isComposing: true })), undefined);
  assert.equal(speedSearchKey("a", key("x", { keyCode: 229 })), undefined);
});

void test("characters typed with AltGr or Option are typed", () => {
  assert.equal(speedSearchKey("", key("@", { ctrlKey: true, altKey: true })), "@");
  assert.equal(speedSearchKey("", key("å", { altKey: true })), "å");
});
