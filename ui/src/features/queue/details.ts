// The queue's Details tab: three titled groups about the selected changeset. A group appears when
// there is something to say, and a fact appears only when it is known.

import type { ChangesetKey } from "../../generated/ChangesetKey";
import type { ChangesetRow } from "../../generated/ChangesetRow";
import type { ReviewFilter } from "../../generated/ReviewFilter";
import { shortTime } from "../../lib/shortTime.ts";
import { behindText } from "../../state/baseBehind.ts";
import { keyId } from "../../state/changesetKey.ts";
import type { Frame } from "../../state/reviewModel.ts";

/** What a fact that is a link opens. */
export type FactLink = "baseChanged";

export interface Fact {
  /** Absent for a fact that is only a statement. */
  label?: string;
  value: string;
  /** The value is a link to this. */
  link?: FactLink;
}

export interface FactGroup {
  title: string;
  facts: Fact[];
}

/** What the review filter lets through, in a few words. */
export function filterText(filter: ReviewFilter): string {
  const parts = [
    ...filter.roots.map((r) => `${r}/`),
    ...filter.include,
    ...filter.exclude.map((glob) => `!${glob}`),
    ...(filter.hideBinary ? ["no binaries"] : []),
  ];
  return parts.length ? parts.join(", ") : "all files";
}

/** What the changes lead to: the live working tree, a pull request's head, or a branch's tip. */
function targetText(key: ChangesetKey, title: string): string {
  switch (key.type) {
    case "workingTree":
      return "live";
    case "pullRequest":
      return `#${key.number}`;
    case "branch":
      return title;
  }
}

function changesetFacts(row: ChangesetRow, { layout, facts }: Frame): Fact[] {
  const version = row.review.state === "open" ? ` · v${row.review.version}` : "";
  const behind = behindText(layout.base);
  return [
    { label: "Base", value: layout.base.commit.shortId },
    ...(behind ? [{ value: behind }] : []),
    { label: "Target", value: `${targetText(row.key, row.title)}${version}` },
    { label: "Filter", value: filterText(facts.filter) },
  ];
}

function reviewFacts(row: ChangesetRow, nowMs: number): Fact[] {
  const review = row.review;
  switch (review.state) {
    case "none":
      return [{ value: "Not started" }];
    case "open":
      return [
        { label: "Started", value: shortTime(review.startedAt, nowMs) },
        { label: "Last reviewed", value: review.lastReviewed === undefined ? "—" : `v${review.lastReviewed}` },
        { label: "Versions", value: String(review.version) },
      ];
    case "finished":
      return [
        { label: "Started", value: shortTime(review.startedAt, nowMs) },
        {
          label: "Finished",
          value: `${shortTime(review.finishedAt, nowMs)}${review.lastReviewed === undefined ? "" : ` · v${review.lastReviewed}`}`,
        },
      ];
  }
}

function progressFacts({ facts }: Frame): Fact[] {
  const c = facts.completion;
  return [
    { label: "Lines", value: `${c.seenLines} / ${c.lines}` },
    { label: "Other changes", value: `${c.acknowledgedItems} / ${c.items}` },
    c.oldBaseLines === 0
      ? { label: "Base changed", value: "—" }
      : { label: "Base changed", value: `${c.oldBaseLines} lines`, link: "baseChanged" },
  ];
}

/** `onView` is the changeset on view as a whole; it describes `row` only when it is that one. */
export function detailGroups(row: ChangesetRow, onView: Frame | undefined, nowMs = Date.now()): FactGroup[] {
  const own = onView && keyId(onView.facts.target.changeset) === keyId(row.key) ? onView : undefined;
  const groups: FactGroup[] = [];
  if (own) groups.push({ title: "Changeset", facts: changesetFacts(row, own) });
  groups.push({ title: "Review", facts: reviewFacts(row, nowMs) });
  // A finished review keeps no marks, so what is seen of it is not known.
  if (own && row.review.state !== "finished") groups.push({ title: "Progress", facts: progressFacts(own) });
  return groups;
}
