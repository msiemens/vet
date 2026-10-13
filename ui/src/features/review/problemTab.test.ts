import assert from "node:assert/strict";
import { test } from "node:test";
import { problemTab } from "./problemTab.ts";

const on = (over: Partial<Parameters<typeof problemTab>[0]> = {}) => ({
  active: "changes",
  changes: "changes",
  file: "file:a.rs",
  open: ["changes"],
  inChanges: true,
  ...over,
});

void test("the Changes tab on screen shows a problem of a file it has, even when the file has a tab open", () => {
  assert.equal(problemTab(on()), "changes");
  assert.equal(problemTab(on({ open: ["changes", "file:a.rs"] })), "changes");
});

void test("a file the Changes tab does not show goes to its open tab, else to a new one", () => {
  assert.equal(problemTab(on({ inChanges: false, open: ["changes", "file:a.rs"] })), "file");
  assert.equal(problemTab(on({ inChanges: false })), "new");
});

void test("from another tab the file's open tab wins over the Changes tab, which is brought to the front otherwise", () => {
  assert.equal(problemTab(on({ active: "file:b.rs", open: ["changes", "file:b.rs", "file:a.rs"] })), "file");
  assert.equal(problemTab(on({ active: "file:a.rs", open: ["changes", "file:a.rs"] })), "file");
  assert.equal(problemTab(on({ active: "file:b.rs", open: ["changes", "file:b.rs"] })), "changesToFront");
});
