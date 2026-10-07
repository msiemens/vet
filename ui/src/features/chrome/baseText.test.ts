import assert from "node:assert/strict";
import { test } from "node:test";
import type { Base } from "../../generated/Base.ts";
import type { BranchInfo } from "../../generated/BranchInfo.ts";
import { baseMenuItems, baseTooltip, orderBranches, parseBaseItem, versionTitle } from "./baseText.ts";

const now = Date.UTC(2026, 9, 6) / 1000;
const branch = (name: string, flags: Partial<BranchInfo> = {}): BranchInfo => ({
  name,
  commit: "c".repeat(40),
  current: false,
  default: false,
  ...flags,
});
const branches = [branch("dev"), branch("feature", { current: true }), branch("main", { default: true })];

void test("branches are ordered current, default, then as listed", () => {
  assert.deepEqual(
    orderBranches(branches).map((b) => b.name),
    ["feature", "main", "dev"],
  );
});

void test("the menu offers each branch as a target, and a commit", () => {
  const items = baseMenuItems(branches, { type: "mergeBase", reference: "main" }, true);
  assert.deepEqual(
    items.filter((i) => i.id.startsWith("branch:")).map((i) => [i.label, i.detail, i.checked]),
    [
      ["feature", "current", false],
      ["main", "default", true],
      ["dev", undefined, false],
    ],
  );
  assert.equal(items.at(-1)?.label, "Pick a commit…");
  assert.equal(/latest|branched off/i.exec(JSON.stringify(items)), null);
});

void test("a changeset's own branch is not offered as its target", () => {
  const items = baseMenuItems(branches, undefined, true, undefined, "dev");
  assert.deepEqual(
    items.filter((i) => i.id.startsWith("branch:")).map((i) => i.label),
    ["feature", "main"],
  );
});

void test("the chosen target says how far its remote has moved on", () => {
  const items = baseMenuItems(branches, { type: "mergeBase", reference: "main" }, true, "11 behind origin/main");
  assert.deepEqual(
    items.filter((i) => i.id.startsWith("branch:")).map((i) => i.detail),
    ["current", "default · 11 behind origin/main", undefined],
  );
});

void test("the commit entry is checked for a commit and disabled until commits are listed", () => {
  const items = baseMenuItems(branches, { type: "commit", id: "a".repeat(40) }, false);
  assert.deepEqual([items.at(-1)?.checked, items.at(-1)?.enabled], [true, false]);
  assert.equal(
    baseMenuItems([], undefined, true).some((i) => i.separator),
    false,
  );
});

void test("items read back as the choice they stand for", () => {
  assert.deepEqual(parseBaseItem("branch:release/1.0"), { type: "mergeBase", reference: "release/1.0" });
  assert.equal(parseBaseItem("commit"), "commits");
});

void test("the tooltip has the choice, the full hash, the subject, the author and the age", () => {
  const base: Base = {
    choice: { type: "mergeBase", reference: "main" },
    commit: { id: "a".repeat(40), shortId: "aaaaaaa", subject: "Fix the thing", author: "Ann", time: now - 3 * 3600 },
    name: "main~3",
  };
  assert.equal(
    baseTooltip(base, "feature", now * 1000),
    ["Where feature branched off main", "a".repeat(40), "Fix the thing", "Ann · 3 hr. ago"].join("\n"),
  );
});

void test("the version says when it settled", () => {
  assert.equal(versionTitle(3, now - 120, now * 1000), "Version 3 · updated 2 min. ago");
});
