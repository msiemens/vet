// How the Comments tool window words what the platform said of a pull request.

import type { Blocker } from "../../generated/Blocker";
import type { PlatformConnection } from "../../generated/PlatformConnection";
import type { PullRequestInfo } from "../../generated/PullRequestInfo";
import type { ReviewerState } from "../../generated/ReviewerState";
import type { Thread } from "../../generated/Thread";

export function stateLabel(info: Pick<PullRequestInfo, "state" | "draft">): string {
  if (info.state === "open") return info.draft ? "Draft" : "Open";
  return info.state === "merged" ? "Merged" : "Closed";
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

/** The messages that `SubmitReview` or `PostDrafts` would post: those only Vet has, in threads of a platform. */
export function draftsToSubmit(threads: readonly Thread[]): number {
  return threads.reduce((n, t) => n + (t.platform ? t.messages.filter((m) => !m.posted).length : 0), 0);
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
