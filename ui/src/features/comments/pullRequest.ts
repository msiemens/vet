// How the Comments tool window words what the platform said of a pull request.

import type { Blocker } from "../../generated/Blocker";
import type { PlatformConnection } from "../../generated/PlatformConnection";
import type { PullRequestInfo } from "../../generated/PullRequestInfo";
import type { ReviewerState } from "../../generated/ReviewerState";
import type { ReviewSubmitted } from "../../generated/ReviewSubmitted";
import { plural } from "../../lib/format.ts";
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
      return plural(blocker.count, "unresolved thread");
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
  return `${plural(count, "draft waits", "drafts wait")}: its code is gone or its thread has an edit to settle.`;
}

/** What is told after a post went through: how many comments were posted, and the vote if one was cast. */
export function postedNote(done: Pick<ReviewSubmitted, "posted" | "skipped" | "voteCast">): string {
  const comments = `${plural(done.posted, "comment")} posted`;
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
