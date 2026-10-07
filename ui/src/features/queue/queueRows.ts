// What the queue and the changeset switcher say about the backend's changeset rows: which tab a row
// belongs to, the words of its Source and Updated columns, and what its progress column shows.

import type { ChangesetRow } from "../../generated/ChangesetRow";
import type { CiState } from "../../generated/CiState";
import type { Completion } from "../../generated/Completion";
import type { LineDelta } from "../../generated/LineDelta";
import type { PullRequestRow } from "../../generated/PullRequestRow";
import type { RowReview } from "../../generated/RowReview";
import type { Target } from "../../generated/Target";
import { shortTime } from "../../lib/shortTime.ts";
import type { IconName } from "../../lib/ui/icons.ts";

/** A row of the queue: a local changeset, or a pull request with what the platform lists of it. */
export type QueueRow = ChangesetRow & { pullRequest?: PullRequestRow };

/** The queue's rows: the local changesets, then the pull requests. */
export function queueRows(local: readonly ChangesetRow[], pullRequests: readonly PullRequestRow[]): QueueRow[] {
  return [...local, ...pullRequests.map(pullRequestRow)];
}

/** A pull request as a row, which has no threads of its own listed: its comments are the platform's count. */
export function pullRequestRow(pullRequest: PullRequestRow): QueueRow {
  return {
    key: pullRequest.key,
    title: pullRequest.title,
    checkedOut: false,
    openThreads: 0,
    review: pullRequest.review,
    pullRequest,
  };
}

/** What a row says of its comments: a pull request's, as the platform counts them, else the open threads. */
export function commentCount(row: QueueRow): number {
  return row.pullRequest?.comments ?? row.openThreads;
}

/** How a changeset is named: a pull request's number stands apart from its title, which may be unknown. */
export interface Label {
  number?: string;
  text: string;
}

export function labelOf(row: QueueRow): Label {
  return row.key.type === "pullRequest" ? { number: `#${row.key.number}`, text: row.title } : { text: row.title };
}

/** What the changeset on view is called in the header. */
export function targetLabel(target: Target): Label {
  const key = target.changeset;
  if (key.type !== "pullRequest") return { text: target.name };
  return target.title === undefined ? { text: `#${key.number}` } : { number: `#${key.number}`, text: target.title };
}

/** The number in what was typed for a pull request: `482` or `#482`; `undefined` for anything else. */
export function pullRequestNumber(typed: string): number | undefined {
  const match = /^#?(\d+)$/.exec(typed.trim());
  const number = match ? Number(match[1]) : 0;
  return number > 0 && Number.isSafeInteger(number) && number <= 0xffff_ffff ? number : undefined;
}

/** How the checks of a pull request's head commit show in the row's status column; none for no checks. */
export function ciIcon(
  ci: CiState | undefined,
): { icon: IconName; tone: "ok" | "failing" | "running"; title: string } | undefined {
  switch (ci) {
    case "success":
      return { icon: "check", tone: "ok", title: "Checks passed" };
    case "failure":
      return { icon: "close", tone: "failing", title: "Checks failing" };
    case "pending":
      return { icon: "timeline", tone: "running", title: "Checks running" };
    default:
      return undefined;
  }
}

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
    case "pullRequest":
      return "pr";
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

/**
 * The Source column: the branch the working tree is on, where a branch lives and how many commits it has,
 * or where a pull request's branch is: the repository itself ("origin") or a fork.
 */
export function sourceText(row: QueueRow): string {
  if (row.pullRequest) return row.pullRequest.fork ?? "origin";
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

/** The lines a row shows: those of the changeset on view when `own` is that, else what the platform lists of a pull request. */
export function shownLines(row: QueueRow, own: LineDelta | undefined): LineDelta | undefined {
  return own && showsOwnLines(row) ? own : row.pullRequest?.lines;
}

/** The Updated column: the working tree is live, a branch changed with its last commit, a pull request when the platform saw it change. */
export function updatedText(row: QueueRow, nowMs = Date.now()): string {
  if (row.pullRequest) return shortTime(row.pullRequest.updatedAt, nowMs);
  if (row.key.type === "workingTree") return "now";
  return row.lastCommit === undefined ? "" : shortTime(row.lastCommit, nowMs);
}

export function tabCounts(rows: readonly QueueRow[]): Record<QueueTab, number> {
  return {
    all: rows.length,
    blocking: rows.filter((r) => inTab(r, "blocking")).length,
    local: rows.filter((r) => inTab(r, "local")).length,
    prs: rows.filter((r) => inTab(r, "prs")).length,
  };
}

/** The rows of a tab whose name or source contain every word of `query`, ignoring case; the order is kept. */
export function filterRows(rows: readonly QueueRow[], tab: QueueTab, query: string): QueueRow[] {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean);
  return rows.filter((row) => {
    if (!inTab(row, tab)) return false;
    const { number = "", text: name } = labelOf(row);
    const text = `${number} ${name} ${sourceText(row)}`.toLowerCase();
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
