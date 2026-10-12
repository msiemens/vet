import assert from "node:assert/strict";
import { test } from "node:test";
import { fileTab, isStale, kindOf, modesOf, pathOf, settleMode, tabAfter, tabOf } from "./tabModel.ts";

void test("a file has one tab id whatever asks for it, and the id tells the path back", () => {
  assert.equal(tabOf("file", "src/a.rs"), fileTab("src/a.rs"));
  assert.equal(pathOf(fileTab("src/a.rs")), "src/a.rs");
  assert.equal(kindOf(fileTab("src/a.rs")), "file");
  // A path with a colon is still one file's.
  assert.equal(pathOf(fileTab("c:/a.rs")), "c:/a.rs");
  assert.equal(pathOf("changes"), undefined);
  assert.equal(pathOf("log:7"), undefined);
});

void test("a changed file offers the diff modes and its text, an unchanged one its text only, a log nothing", () => {
  assert.deepEqual(modesOf("file", true), ["unified", "split", "file"]);
  assert.deepEqual(modesOf("file", false), ["file"]);
  assert.deepEqual(modesOf("changes", false), ["unified", "split"]);
  assert.deepEqual(modesOf("log", false), []);
});

void test("a tab asked for without a mode keeps its own, and a new tab of a changed file opens in the last diff mode", () => {
  assert.equal(settleMode({ changed: true, current: "file", last: "split" }), "file");
  assert.equal(settleMode({ changed: true, current: "unified", last: "split" }), "unified");
  assert.equal(settleMode({ changed: true, last: "split" }), "split");
});

void test("Jump to Source asks for the text of a file, and a mode change for the mode chosen", () => {
  assert.equal(settleMode({ changed: true, want: "file", current: "unified", last: "unified" }), "file");
  assert.equal(settleMode({ changed: true, want: "split", current: "file", last: "unified" }), "split");
});

void test("an unchanged file shows its text whatever is asked", () => {
  assert.equal(settleMode({ changed: false, last: "split" }), "file");
  assert.equal(settleMode({ changed: false, want: "unified", current: "split", last: "split" }), "file");
});

void test("only a diff of a file that is no longer changed is stale", () => {
  const none = () => false;
  const a = () => true;
  assert.equal(isStale(fileTab("a.rs"), "split", none), true);
  assert.equal(isStale(fileTab("a.rs"), "split", a), false);
  assert.equal(isStale(fileTab("a.rs"), "file", none), false);
  assert.equal(isStale("changes", "unified", none), false);
  assert.equal(isStale("log:1", undefined, none), false);
});

void test("stepping through the tabs wraps around both ends", () => {
  const open = ["changes", "a", "b"];
  assert.equal(tabAfter(open, "changes", 1), "a");
  assert.equal(tabAfter(open, "b", 1), "changes");
  assert.equal(tabAfter(open, "changes", -1), "b");
  assert.equal(tabAfter(open, "a", -1), "changes");
  assert.equal(tabAfter(["changes"], "changes", 1), "changes");
});
