// What the queue and the changeset switcher say about the backend's changeset rows: which tab a row
// belongs to, the words of its Source and Updated columns, and what its progress column shows.

import type { ChangesetRow } from "../../generated/ChangesetRow";
import type { Completion } from "../../generated/Completion";
import type { RowReview } from "../../generated/RowReview";
import { shortTime } from "../../lib/shortTime.ts";

export type QueueTab = "all" | "blocking" | "local" | "prs";

export const TABS: { id: QueueTab; label: string }[] = [
  { id: "all", label: "All" },
  { id: "blocking", label: "Blocking" },
  { id: "local", label: "Local" },
  { id: "prs", label: "Pull requests" },
];

/** The changeset switcher's filters are the queue's tabs, with the short name for pull requests. */
export const SWITCHER_FILTERS: { id: QueueTab; label: string }[] = TABS.map((t) =>
  t.id === "prs" ? { ...t, label: "PRs" } : t,
);

/** Where a row is listed besides All. A new kind of changeset is told here which one it is. */
export function categoryOf(row: ChangesetRow): "local" | "pr" {
  switch (row.key.type) {
    case "workingTree":
    case "branch":
      return "local";
  }
}

function inTab(row: ChangesetRow, tab: QueueTab): boolean {
  switch (tab) {
    case "all":
      return true;
    case "blocking":
      // What blocks a merge is a thread's flag, which no thread carries yet.
      return false;
    case "local":
      return categoryOf(row) === "local";
    case "prs":
      return categoryOf(row) === "pr";
  }
}

/** The Source column: the branch the working tree is on, or where a branch lives and how many commits it has. */
export function sourceText(row: ChangesetRow): string {
  if (row.key.type === "workingTree") return row.branch ?? "";
  const ahead = row.divergence?.ahead;
  return ahead === undefined ? "local" : `local · ${ahead} ${ahead === 1 ? "commit" : "commits"}`;
}

/**
 * Whether the changeset on view's lines are shown on `row`. They are counted against the base the
 * review uses, so a branch with no commits of its own on the default branch cannot show them.
 */
export function showsOwnLines(row: ChangesetRow): boolean {
  return row.divergence?.ahead !== 0;
}

/** The Updated column: the working tree is live, a branch changed with its last commit. */
export function updatedText(row: ChangesetRow, nowMs = Date.now()): string {
  if (row.key.type === "workingTree") return "now";
  return row.lastCommit === undefined ? "" : shortTime(row.lastCommit, nowMs);
}

export function tabCounts(rows: readonly ChangesetRow[]): Record<QueueTab, number> {
  return {
    all: rows.length,
    blocking: rows.filter((r) => inTab(r, "blocking")).length,
    local: rows.filter((r) => inTab(r, "local")).length,
    prs: rows.filter((r) => inTab(r, "prs")).length,
  };
}

/** The rows of a tab whose title or source contain every word of `query`, ignoring case; the order is kept. */
export function filterRows(rows: readonly ChangesetRow[], tab: QueueTab, query: string): ChangesetRow[] {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean);
  return rows.filter((row) => {
    if (!inTab(row, tab)) return false;
    const text = `${row.title} ${sourceText(row)}`.toLowerCase();
    return words.every((w) => text.includes(w));
  });
}

/** What the progress column shows of a row. */
export type Reviewed =
  /** Not started. */
  | { kind: "none" }
  /** A review is open and its progress is not known here. */
  | { kind: "started" }
  | { kind: "progress"; seen: number; oldBase: number }
  /** Finished; `percent` is known when it was complete. */
  | { kind: "finished"; percent?: number };

/** `completion` is the progress of the changeset on view, which is the only one whose lines are known. */
export function reviewedOf(review: RowReview, completion?: Completion): Reviewed {
  switch (review.state) {
    case "none":
      return { kind: "none" };
    case "open":
      return completion
        ? { kind: "progress", seen: completion.seenPercent, oldBase: completion.oldBasePercent }
        : { kind: "started" };
    case "finished":
      return review.complete ? { kind: "finished", percent: 100 } : { kind: "finished" };
  }
}
