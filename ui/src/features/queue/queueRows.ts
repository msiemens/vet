// What the queue and the changeset switcher say about the backend's changeset rows: which tab a row
// belongs to, the words of its Source and Updated columns, and what its progress column shows.

import type { ChangesetRow } from "../../generated/ChangesetRow";
import type { PullRequestRow } from "../../generated/PullRequestRow";
import type { QueueStatus } from "../../generated/QueueStatus";
import type { Target } from "../../generated/Target";
import { plural } from "../../lib/format.ts";
import { shortTime } from "../../lib/shortTime.ts";
import type { QueueRow } from "../../state/queueEntries.ts";

// The rows themselves and what the Changes and Reviewed columns show of them live in the queue
// store's pure module; the switcher and the panel read the same functions through here.
export {
  type Changes,
  type QueueEntry,
  type QueueRow,
  type Reviewed,
  changesOf,
  pullRequestRow,
  queueRows,
  reviewedOf,
} from "../../state/queueEntries.ts";

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
  if (row.key.type === "pullRequest") return { number: `#${row.key.number}`, text: row.title };
  // The default branch is offered for the commits it has not pushed.
  return { text: row.unpushed ? `${row.title}, ${row.unpushed} unpushed` : row.title };
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

/** The tab a stored value names; the first for anything else. */
export function parseTab(value: unknown): QueueTab {
  return TABS.find((t) => t.id === value)?.id ?? TABS[0].id;
}

/** The tab `delta` places after `tab`, wrapping around the row. */
export function stepTab(tab: QueueTab, delta: number): QueueTab {
  const at = TABS.findIndex((t) => t.id === tab);
  return TABS[(((at + delta) % TABS.length) + TABS.length) % TABS.length].id;
}

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
  return ahead === undefined ? "local" : `local · ${plural(ahead, "commit")}`;
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
  if (row.pullRequest) return { name: row.pullRequest.authorName, avatar: row.pullRequest.authorAvatar, muted: false };
  return row.author === undefined ? undefined : { name: row.author, muted: true };
}

/** The Updated column: the working tree is live, a branch changed with its last commit, a pull request when the platform saw it change. */
export function updatedText(row: QueueRow, nowMs: number): string {
  if (row.pullRequest) return shortTime(row.pullRequest.updatedAt, nowMs);
  if (row.key.type === "workingTree") return "now";
  return row.lastCommit === undefined ? "" : shortTime(row.lastCommit, nowMs);
}

const STATUS_ORDER: readonly QueueStatus[] = ["blocked", "changesRequested", "waitingForYou", "approved", "draft"];

/** Where a row sorts by its status: the statuses most in need of attention first; nowhere without one. */
export function statusOrder(row: QueueRow): number | undefined {
  const status = row.pullRequest?.status;
  return status && STATUS_ORDER.indexOf(status);
}

/** When a row changed, in seconds, for sorting: the live working tree is the newest. */
export function updatedAt(row: QueueRow): number | undefined {
  if (row.pullRequest) return row.pullRequest.updatedAt;
  return row.key.type === "workingTree" ? Infinity : row.lastCommit;
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
export function filterRows<R extends QueueRow>(rows: readonly R[], tab: QueueTab, query: string): R[] {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean);
  return rows.filter((row) => {
    if (!inTab(row, tab)) return false;
    const { number = "", text: name } = labelOf(row);
    const text = `${number} ${name} ${sourceText(row)}`.toLowerCase();
    return words.every((w) => text.includes(w));
  });
}
