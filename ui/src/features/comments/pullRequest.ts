// How the Comments tool window words what the platform said of a pull request.

import type { Blocker } from "../../generated/Blocker";
import type { PendingVote } from "../../generated/PendingVote";
import type { PlatformConnection } from "../../generated/PlatformConnection";
import type { PullRequestInfo } from "../../generated/PullRequestInfo";
import type { ReviewerState } from "../../generated/ReviewerState";
import type { ReviewSubmitted } from "../../generated/ReviewSubmitted";
import type { Thread } from "../../generated/Thread";
import type { ThreadMessage } from "../../generated/ThreadMessage";
import type { StatusTone } from "../../lib/ui/statusTone.ts";

export function stateLabel(info: Pick<PullRequestInfo, "state" | "draft">): string {
  if (info.state === "open") return info.draft ? "Draft" : "Open";
  return info.state === "merged" ? "Merged" : "Closed";
}

/** The dot beside a pull request's state: open is well, closed is not, merged is its own. */
export function stateTone(state: PullRequestInfo["state"]): StatusTone {
  return state === "open" ? "ok" : state === "merged" ? "note" : "warn";
}

export function blockerText(blocker: Blocker): string {
  switch (blocker.type) {
    case "threads":
      return `${blocker.count} unresolved ${blocker.count === 1 ? "thread" : "threads"}`;
    case "checks":
      return "checks";
    case "changesRequested":
      return "changes requested";
  }
}

export function reviewerMark(state: ReviewerState): string {
  switch (state) {
    case "approved":
      return "✓";
    case "changesRequested":
      return "✗";
    case "commented":
      return "…";
    case "waiting":
      return "○";
  }
}

/** What is said of the drafts that were not posted because their code is gone or their thread has an edit to settle. */
export function waitingNote(count: number): string {
  return `${count} ${count === 1 ? "draft waits" : "drafts wait"}: its code is gone or its thread has an edit to settle.`;
}

/** The messages that `SubmitReview` or `PostDrafts` would post: the drafts and failed ones of threads of a platform. */
export function draftsToSubmit(threads: readonly Thread[]): number {
  const waits = (m: ThreadMessage) => m.state.type === "draft" || m.state.type === "failed";
  return threads.reduce((n, t) => n + (t.platform ? t.messages.filter(waits).length : 0), 0);
}

/** Whether a post would send the staged vote: not when there is none, nor while it is on its way. */
export const voteWaits = (vote: PendingVote | null | undefined): boolean =>
  vote != null && vote.state.type !== "posting";

/** What a post would send: the comments that wait and the staged vote. */
export const waitingToPost = (threads: readonly Thread[], vote: PendingVote | null | undefined): number =>
  draftsToSubmit(threads) + (voteWaits(vote) ? 1 : 0);

/** What is told after a post went through: how many comments were posted, and the vote if one was cast. */
export function postedNote(done: Pick<ReviewSubmitted, "posted" | "skipped" | "voteCast">): string {
  const comments = `${done.posted} ${done.posted === 1 ? "comment" : "comments"} posted`;
  const told = done.posted === 0 && done.voteCast ? "Vote cast" : done.voteCast ? `${comments}, vote cast` : comments;
  return done.skipped > 0 ? `${told}. ${waitingNote(done.skipped)}` : told;
}

/** Why the review cannot be submitted now, if it cannot. */
export function submitBlock(connection: PlatformConnection | undefined): string | undefined {
  switch (connection?.type) {
    case "signedOut":
      return "Sign in to submit the review";
    case "tokenExpired":
      return "The token expired: sign in again to submit the review";
    default:
      return undefined;
  }
}
