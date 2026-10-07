import assert from "node:assert/strict";
import { test } from "node:test";
import type { ChangesetRow } from "../../generated/ChangesetRow";
import { changesetItems, rowDetail } from "./changesetMenu.ts";

const NOW = 1_000_000 * 1000;
const tree: ChangesetRow = {
  key: { type: "workingTree" },
  title: "Working tree",
  checkedOut: false,
  review: { state: "none" },
};
const feature: ChangesetRow = {
  key: { type: "branch", name: "feature" },
  title: "feature",
  checkedOut: false,
  divergence: { ahead: 3, behind: 1 },
  lastCommit: 1_000_000 - 2 * 86400,
  review: { state: "open", version: 2 },
};

void test("a branch row says how far it is from the default branch, how old it is and whether it is under review", () => {
  assert.equal(rowDetail(feature, NOW), "↑3 ↓1 · 2 days ago · reviewing · v2");
  assert.equal(
    rowDetail({ ...feature, checkedOut: true, review: { state: "none" } }, NOW),
    "↑3 ↓1 · 2 days ago · checked out",
  );
});

void test("the working tree has nothing to say until it is reviewed", () => {
  assert.equal(rowDetail(tree, NOW), "");
  assert.equal(rowDetail({ ...tree, review: { state: "finished", complete: true } }, NOW), "reviewed");
});

void test("the item of the changeset on view is checked and each is named by its key", () => {
  const items = changesetItems([tree, feature], { type: "branch", name: "feature" }, NOW);
  assert.deepEqual(
    items.map((i) => [i.id, i.checked]),
    [
      ["workingTree", false],
      ["branch:feature", true],
    ],
  );
  assert.equal(items[0].detail, undefined);
});
