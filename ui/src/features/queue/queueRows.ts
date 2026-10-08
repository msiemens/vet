// What the queue and the changeset switcher say about the backend's changeset rows: which tab a row
// belongs to, the words of its Source and Updated columns, and what its progress column shows.

import type { ChangesetRow } from "../../generated/ChangesetRow";
import type { Completion } from "../../generated/Completion";
import type { LineDelta } from "../../generated/LineDelta";
import type { PullRequestRow } from "../../generated/PullRequestRow";
import type { QueueStatus } from "../../generated/QueueStatus";
import type { RowReview } from "../../generated/RowReview";
import type { Target } from "../../generated/Target";
import { shortTime } from "../../lib/shortTime.ts";

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

/** What the Changes column says of a row. */
export type Changes =
  /** The backend has not counted the lines yet. */
  | { kind: "pending" }
  /** There are none: nothing changed, or the row has no lines to count. */
  | { kind: "none" }
  | { kind: "lines"; lines: LineDelta };

/**
 * The lines of a row, from the most current source: `own`, the live manifest, when the row is the changeset
 * on view; else what the platform lists of a pull request; else `counted`, what the backend counted between
 * the commits of the row's base and tip (`null`: there are none to count). The working tree has its manifest
 * only.
 */
export function changesOf(row: QueueRow, own: LineDelta | undefined, counted: LineDelta | null | undefined): Changes {
  const known = own ?? row.pullRequest?.lines ?? counted;
  if (known) return known.added + known.removed > 0 ? { kind: "lines", lines: known } : { kind: "none" };
  return row.key.type === "workingTree" || counted === null ? { kind: "none" } : { kind: "pending" };
}

/** The label of a pull request's status chip. */
export function statusText(status: QueueStatus): string {
  switch (status) {
    case "draft":
      return "Draft";
    case "blocked":
      return "Blocked";
    case "changesRequested":
      return "Changes requested";
    case "waitingForYou":
      return "Waiting for you";
    case "approved":
      return "Approved";
  }
}

/**
 * What a pull request's status chip says on hover: who asked for changes and how, one reviewer a line, for the
 * one "Changes requested" chip that rejections, waits for the author and requested changes share.
 */
export function statusTitle(pullRequest: PullRequestRow): string | undefined {
  if (pullRequest.status !== "changesRequested" || pullRequest.changesRequested.length === 0) return undefined;
  return pullRequest.changesRequested
    .map(({ reviewer, how }) => {
      switch (how) {
        case "requested":
          return `Changes requested by ${reviewer}`;
        case "rejected":
          return `Rejected by ${reviewer}`;
        case "waitingForAuthor":
          return `Waiting for the author (${reviewer})`;
      }
    })
    .join("\n");
}

export interface Author {
  name: string;
  /** The address of the platform's picture of the author, when it has one. */
  avatar?: string;
  /** The author of a branch's tip commit rather than a pull request's. */
  muted: boolean;
}

/** The Author column: a pull request's author, a branch's tip commit's author, and nobody for the working tree. */
export function authorOf(row: QueueRow): Author | undefined {
  if (row.pullRequest) return { name: row.pullRequest.author, avatar: row.pullRequest.authorAvatar, muted: false };
  return row.author === undefined ? undefined : { name: row.author, muted: true };
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

/**
 * `completion` is the live progress of the changeset on view; any other row has the progress stored with
 * its review, if that was ever measured.
 */
export function reviewedOf(
  review: RowReview,
  completion?: Pick<Completion, "seenPercent" | "oldBasePercent">,
): Reviewed {
  switch (review.state) {
    case "none":
      return { kind: "none" };
    case "open": {
      const progress = completion ?? review.progress;
      return progress
        ? { kind: "progress", seen: progress.seenPercent, oldBase: progress.oldBasePercent }
        : { kind: "started" };
    }
    case "finished":
      return review.complete ? { kind: "finished", percent: 100 } : { kind: "finished" };
  }
}
