import assert from "node:assert/strict";
import { test } from "node:test";
import {
  changesTitle,
  commitOf,
  fileTab,
  isStale,
  kindOf,
  modesOf,
  needsCommitName,
  pathOf,
  scrollToReveal,
  settleMode,
  tabAfter,
} from "./tabModel.ts";

void test("the Changes tab's tooltip carries the file count", () => {
  assert.match(changesTitle(34), /^34 changed files$/);
  assert.match(changesTitle(1), /^1 changed file$/);
  assert.match(changesTitle(3, "fix: retry"), /^fix: retry\n3 changed files$/);
});

void test("a file opened at a commit is another tab than the file, and the id tells both back", () => {
  const sha = "a".repeat(40);
  const pinned = fileTab("src/a.rs", sha);
  assert.notEqual(pinned, fileTab("src/a.rs"));
  assert.notEqual(pinned, fileTab("src/a.rs", "b".repeat(40)));
  assert.equal(kindOf(pinned), "file");
  assert.equal(pathOf(pinned), "src/a.rs");
  assert.equal(commitOf(pinned), sha);
  assert.equal(commitOf(fileTab("src/a.rs")), undefined);
  // A path with colons is still the file's.
  assert.equal(pathOf(fileTab("c:/a:b.rs", sha)), "c:/a:b.rs");
});

void test("the commit is named in the tabs of a file open more than once, and only in the pinned ones", () => {
  const [x, y] = ["x".repeat(40), "y".repeat(40)];
  const open = [fileTab("a.rs", x), fileTab("b.rs", x), fileTab("a.rs", y), fileTab("c.rs"), fileTab("c.rs", x)];
  assert.deepEqual(
    open.map((id) => needsCommitName(open, id)),
    [true, false, true, false, true],
  );
});

void test("a file's tab id tells the path back", () => {
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

void test("revealing a tab scrolls the least that shows it whole", () => {
  // View [100, 300): a tab in view stays put, one past either edge moves just far enough.
  assert.equal(scrollToReveal(100, 200, 150, 80), 100);
  assert.equal(scrollToReveal(100, 200, 250, 80), 130);
  assert.equal(scrollToReveal(100, 200, 60, 80), 60);
  // A tab wider than the view shows its start.
  assert.equal(scrollToReveal(100, 200, 250, 300), 250);
});
