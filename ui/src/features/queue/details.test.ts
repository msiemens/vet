import assert from "node:assert/strict";
import { test } from "node:test";
import type { ChangesetRow } from "../../generated/ChangesetRow";
import type { ReviewFilter } from "../../generated/ReviewFilter";
import type { Frame } from "../../state/reviewModel.ts";
import { detailGroups, filterText } from "./details.ts";

const NOW = new Date(2026, 9, 7, 15, 30).getTime();
const stamp = (h: number, m: number) => new Date(2026, 9, 7, h, m).getTime() / 1000;
const open: ChangesetRow = {
  key: { type: "workingTree" },
  title: "Working tree",
  checkedOut: false,
  openThreads: 0,
  review: { state: "open", version: 17, startedAt: stamp(9, 52), lastReviewed: 14 },
};
const manifest = {
  layout: { base: { commit: { shortId: "a41c9e2" } } },
  facts: {
    target: { changeset: { type: "workingTree" }, name: "Working tree" },
    filter: { roots: [], include: [], exclude: [], hideBinary: false, reviewGenerated: false },
    completion: { lines: 77, seenLines: 47, items: 3, acknowledgedItems: 1, oldBaseLines: 5 },
  },
} as Frame;

const facts = (groups: ReturnType<typeof detailGroups>) =>
  groups.map((g) => [g.title, g.facts.map((f) => [f.label, f.value])]);

void test("the changeset on view has all three groups", () => {
  assert.deepEqual(facts(detailGroups(open, manifest, NOW)), [
    [
      "Changeset",
      [
        ["Base", "a41c9e2"],
        ["Target", "live · v17"],
        ["Filter", "all files"],
      ],
    ],
    [
      "Review",
      [
        ["Started", "09:52"],
        ["Last reviewed", "v14"],
        ["Versions", "17"],
      ],
    ],
    [
      "Progress",
      [
        ["Lines", "47 / 77"],
        ["Other changes", "1 / 3"],
        ["Base changed", "5 lines"],
      ],
    ],
  ]);
});

void test("a pull request's target is its number, not its title", () => {
  const key = { type: "pullRequest", remote: "github.com/acme/app", number: 482 } as const;
  const row: ChangesetRow = { ...open, key, title: "Add idempotency keys", review: { state: "none" } };
  const viewed = { ...manifest, facts: { ...manifest.facts, target: { changeset: key, name: "#482" } } } as Frame;
  const target = detailGroups(row, viewed, NOW)[0].facts.find((f) => f.label === "Target");
  assert.equal(target?.value, "#482");
});

void test("base changed lines are a link to them, and nothing to open without any", () => {
  const baseChanged = (m: Frame) => detailGroups(open, m, NOW)[2].facts.find((f) => f.label === "Base changed");
  assert.deepEqual(baseChanged(manifest), { label: "Base changed", value: "5 lines", link: "baseChanged" });
  const none = {
    ...manifest,
    facts: { ...manifest.facts, completion: { ...manifest.facts.completion, oldBaseLines: 0 } },
  };
  assert.deepEqual(baseChanged(none), { label: "Base changed", value: "—" });
});

void test("a changeset that is not on view has only what the list knows of its review", () => {
  const other = { ...open, key: { type: "branch", name: "feature" }, title: "feature" } as ChangesetRow;
  assert.deepEqual(
    detailGroups(other, manifest, NOW).map((g) => g.title),
    ["Review"],
  );
});

void test("a changeset not yet reviewed says so and nothing else about the review", () => {
  const groups = detailGroups({ ...open, review: { state: "none" } }, manifest, NOW);
  assert.deepEqual(groups[1], { title: "Review", facts: [{ value: "Not started" }] });
});

void test("a finished review says when, and shows no progress", () => {
  const finished = {
    ...open,
    review: { state: "finished", complete: true, startedAt: stamp(9, 52), finishedAt: stamp(11, 40), lastReviewed: 14 },
  } as ChangesetRow;
  const groups = detailGroups(finished, manifest, NOW);
  assert.deepEqual(
    groups.map((g) => g.title),
    ["Changeset", "Review"],
  );
  assert.deepEqual(groups[1].facts[1], { label: "Finished", value: "11:40 · v14" });
});

void test("the filter reads as what it lets through", () => {
  const filter: ReviewFilter = {
    roots: ["src/sync"],
    include: [],
    exclude: ["*.lock"],
    hideBinary: true,
    reviewGenerated: false,
  };
  assert.equal(filterText(filter), "src/sync/, !*.lock, no binaries");
});
