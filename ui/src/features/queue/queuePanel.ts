// What the queue's side panel says of the selected changeset, in words: why it is blocked, where the
// viewer's review stands and who reviews it. Plain data in, plain text out.

import type { Completion } from "../../generated/Completion";
import type { PlatformKind } from "../../generated/PlatformKind";
import type { PullRequestInfo } from "../../generated/PullRequestInfo";
import type { PullRequestRow } from "../../generated/PullRequestRow";
import type { Reviewer } from "../../generated/Reviewer";
import type { RowReview } from "../../generated/RowReview";
import type { Thread } from "../../generated/Thread";
import type { VersionInfo } from "../../generated/VersionInfo";
import { type QueueRow, authorOf, sourceText, updatedText } from "./queueRows.ts";

/** One clause of a reason; `bold` is the name it is about. */
export interface Part {
  bold?: string;
  text: string;
}

/** What the header says beside the status chip. */
export interface Reason {
  /** `bad` for what holds the merge back, `muted` for where things stand. */
  tone: "bad" | "muted";
  parts: Part[];
  /** The platform holds the merge back by rules it does not name; the page of the pull request shows them. */
  protection: boolean;
}

/** One item of the header's line under the title. */
export type MetaItem =
  | { type: "author"; name: string; avatar?: string; muted: boolean }
  | { type: "mono"; text: string }
  | { type: "text"; text: string };

/**
 * The line under the title: for a pull request its author, its branches and when it changed; for the
 * working tree where its changes live; for a branch its author, where it is and when it changed.
 */
export function metaOf(row: QueueRow, files: number | undefined, nowMs = Date.now()): MetaItem[] {
  if (row.key.type === "workingTree") {
    return [
      { type: "text", text: "Live changes on" },
      ...(row.branch ? [{ type: "mono" as const, text: row.branch }] : []),
      ...(files === undefined
        ? []
        : [{ type: "text" as const, text: `${plural(files, "file", "files")}, not committed` }]),
    ];
  }
  const author = authorOf(row);
  const updated = updatedText(row, nowMs);
  return [
    ...(author ? [{ type: "author" as const, ...author }] : []),
    row.pullRequest
      ? { type: "mono", text: `${row.pullRequest.sourceBranch} → ${row.pullRequest.targetBranch}` }
      : { type: "text", text: sourceText(row) },
    ...(updated ? [{ type: "text" as const, text: `updated ${updated}` }] : []),
  ];
}

const plural = (n: number, one: string, many: string) => `${n} ${n === 1 ? one : many}`;

const number = new Intl.NumberFormat("en");

/** How many messages someone else wrote after the viewer last looked in threads the viewer is in. */
export function repliesForYou(threads: readonly Thread[], isNew: (message: Thread["messages"][number]) => boolean) {
  return threads.filter((t) => t.messages.some((m) => m.mine)).reduce((n, t) => n + t.messages.filter(isNew).length, 0);
}

function approvedBy(reviewers: readonly Reviewer[]): string[] {
  return reviewers.filter((r) => r.state === "approved").map((r) => r.name);
}

/**
 * Why the pull request is where it is. A blocked one names what holds it back: the failed checks, the
 * unresolved threads, who asked for changes. When none of that is known and the platform is GitHub, it is
 * branch protection, which GitHub does not break down; the other statuses say where things stand.
 */
export function reasonOf(input: {
  pull: Pick<PullRequestRow, "status" | "ci" | "changesRequested" | "unresolvedThreads">;
  info?: Pick<PullRequestInfo, "reviewers">;
  platform?: PlatformKind;
  /** The names of the head commit's failed checks. */
  failed: readonly string[];
  /** Replies to the viewer's threads they have not read. */
  unread: number;
}): Reason {
  const { pull, info, platform, failed, unread } = input;
  const asked = pull.changesRequested.map((c) => c.reviewer);
  const changes: Part[] = asked.length ? [{ text: `Changes requested by ${asked.join(", ")}` }] : [];
  if (pull.status === "blocked") {
    const parts: Part[] = [
      ...failed.map((name) => ({ bold: name, text: " failed" })),
      ...(failed.length === 0 && pull.ci === "failure" ? [{ text: "Checks failed" }] : []),
      ...(pull.unresolvedThreads > 0
        ? [{ text: plural(pull.unresolvedThreads, "unresolved thread", "unresolved threads") }]
        : []),
      ...changes,
    ];
    return { tone: "bad", parts, protection: parts.length === 0 && platform === "github" };
  }
  const checks = { success: "Checks pass", failure: "Checks failing", pending: "Checks running" } as const;
  const ci = pull.ci === "success" || pull.ci === "failure" || pull.ci === "pending" ? checks[pull.ci] : undefined;
  const approved = approvedBy(info?.reviewers ?? []);
  const parts: Part[] = [
    ...(ci ? [{ text: ci }] : []),
    ...changes,
    ...(approved.length ? [{ text: `${approved.join(", ")} approved` }] : []),
    ...(unread > 0 ? [{ text: `${plural(unread, "reply", "replies")} for you` }] : []),
  ];
  return { tone: "muted", parts, protection: false };
}

/** What the Your review section says. */
export interface ReviewSummary {
  /** "Not started", "In progress" or "Finished". */
  state: string;
  /** The size of the work or how far the viewer is. */
  detail: string;
  /** How far the viewer is, when a review is open. */
  percent?: number;
  /** The meter: lines seen and lines seen against an older base, in percent. */
  seen: number;
  oldBase: number;
  /** The review is asked of the viewer. */
  asked: boolean;
  /** The base the changes are compared with. */
  base?: string;
  /** What was pushed since the viewer last marked something. */
  pushes?: string;
  /** How many lines were seen against an older base. */
  oldBaseNote?: string;
}

export function reviewSummary(input: {
  review: RowReview;
  /** The live numbers of the changeset on view. */
  completion?: Completion;
  files?: number;
  asked: boolean;
  base?: { branch?: string; shortId: string };
  versions: readonly Pick<VersionInfo, "n" | "files">[];
  /** What a new version is called, singular and plural: the pushes of a pull request. */
  noun: readonly [string, string];
}): ReviewSummary {
  const { review, completion, files, asked, base, versions, noun } = input;
  const state = { none: "Not started", open: "In progress", finished: "Finished" }[review.state];
  const progress = completion ?? (review.state === "open" ? review.progress : undefined);
  let detail = "";
  if (review.state === "finished") detail = review.complete ? "complete" : "with changes unseen";
  else if (completion && review.state === "none") {
    detail = `${number.format(completion.lines)} changed ${completion.lines === 1 ? "line" : "lines"}`;
    if (files !== undefined) detail += ` in ${plural(files, "file", "files")}`;
  } else if (completion) {
    detail = `${number.format(completion.seenLines)} of ${number.format(completion.lines)} lines seen`;
  }
  const finishedWhole = review.state === "finished" && review.complete;
  const summary: ReviewSummary = {
    state,
    detail,
    seen: finishedWhole ? 100 : (progress?.seenPercent ?? 0),
    oldBase: finishedWhole ? 0 : (progress?.oldBasePercent ?? 0),
    asked,
  };
  if (review.state === "open" && progress) summary.percent = Math.round(progress.seenPercent);
  if (base) summary.base = `Base ${base.branch ? `${base.branch} · ` : ""}${base.shortId}`;
  const reviewed = review.state === "open" ? review.lastReviewed : undefined;
  if (review.state === "open" && reviewed !== undefined && reviewed < review.version) {
    const since = versions.filter((v) => v.n > reviewed);
    const changed = new Set(since.flatMap((v) => v.files.map((f) => f.path))).size;
    const older = completion?.oldBaseLines ?? 0;
    summary.pushes =
      `${plural(review.version - reviewed, ...noun)} since you last reviewed ` +
      `(v${reviewed} → v${review.version}): ${plural(changed, "file", "files")} changed` +
      (older > 0 ? `, and ${plural(older, "line", "lines")} you had seen changed again` : "");
  }
  if (completion && completion.oldBaseLines > 0) {
    summary.oldBaseNote = `${plural(completion.oldBaseLines, "line was", "lines were")} seen against an older base.`;
  }
  return summary;
}

/** A reviewer's standing in a few words. */
export function reviewerLabel(state: Reviewer["state"]): string {
  switch (state) {
    case "approved":
      return "approved";
    case "changesRequested":
      return "changes requested";
    case "commented":
      return "commented";
    case "waiting":
      return "waiting";
  }
}

/** What the folded Reviewers section says: who decided and how many still wait. */
export function reviewersSummary(reviewers: readonly Reviewer[]): string {
  const decided = reviewers
    .filter((r) => r.state === "approved" || r.state === "changesRequested")
    .map((r) => `${r.name} ${r.state === "approved" ? "✓" : "✗"}`);
  const waiting = reviewers.filter((r) => r.state === "waiting").length;
  return [...decided, ...(waiting ? [`${waiting} waiting`] : [])].join(" · ");
}
