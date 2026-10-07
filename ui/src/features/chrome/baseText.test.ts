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

void test("the menu offers each branch's latest, where the current one branched off, and a commit", () => {
  const items = baseMenuItems(branches, { type: "branch", name: "main" }, true);
  const branchItems = items.filter((i) => i.id.startsWith("branch:"));
  assert.deepEqual(
    branchItems.map((i) => [i.label, i.detail, i.checked]),
    [
      ["feature (latest)", "current", false],
      ["main (latest)", "default", true],
      ["dev (latest)", undefined, false],
    ],
  );
  const off = items.find((i) => i.id === "mergeBase");
  assert.equal(off?.label, "Where feature branched off");
  assert.deepEqual(
    off.children?.map((i) => i.label),
    ["main", "dev"],
  );
  assert.equal(items.at(-1)?.label, "Pick a commit…");
  assert.equal(/pin|HEAD at open/i.exec(JSON.stringify(items)), null);
});

void test("the chosen branch says how far its remote has moved on", () => {
  const items = baseMenuItems(branches, { type: "branch", name: "main" }, true, "11 behind origin/main");
  assert.deepEqual(
    items.filter((i) => i.id.startsWith("branch:")).map((i) => i.detail),
    ["current", "default · 11 behind origin/main", undefined],
  );
});

void test("the checks follow the choice, and a detached HEAD is named as such", () => {
  const merge = baseMenuItems(branches, { type: "mergeBase", reference: "dev" }, false);
  assert.equal(merge.find((i) => i.id === "mergeBase")?.checked, true);
  assert.equal(merge.find((i) => i.id === "mergeBase")?.children?.find((i) => i.id === "mb:dev")?.checked, true);
  assert.equal(merge.find((i) => i.id === "commit")?.enabled, false);

  const detached = baseMenuItems([branch("main")], undefined, true);
  assert.equal(detached.find((i) => i.id === "mergeBase")?.label, "Where HEAD branched off");
  assert.equal(
    baseMenuItems([], undefined, true).some((i) => i.separator),
    false,
  );
});

void test("items read back as the choice they stand for", () => {
  assert.deepEqual(parseBaseItem("branch:main"), { type: "branch", name: "main" });
  assert.deepEqual(parseBaseItem("mb:release/1.0"), { type: "mergeBase", reference: "release/1.0" });
  assert.equal(parseBaseItem("commit"), "commits");
  assert.equal(parseBaseItem("mergeBase"), undefined);
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
