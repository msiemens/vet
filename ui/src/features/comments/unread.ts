// What the Comments tool's rail icon counts: what others said on the pull request as a whole since
// the viewer last had the tool window open. Comments on lines show at their lines and do not count.

import type { ChangesetKey } from "../../generated/ChangesetKey";
import type { PullRequestInfo } from "../../generated/PullRequestInfo";
import type { Thread } from "../../generated/Thread";

/** Unix seconds of each comment of someone else's on the pull request as a whole: its conversation entries (comments and reviews), and the messages of its threads. */
function othersSaid(info: Pick<PullRequestInfo, "conversation"> | undefined, threads: readonly Thread[]): number[] {
  const conversation = (info?.conversation ?? []).filter((e) => !e.kind.mine).map((e) => e.at);
  const inThreads = threads
    .filter((t) => t.kind === "changeset")
    .flatMap((t) => t.messages.filter((m) => !m.mine).map((m) => m.at));
  return [...conversation, ...inThreads];
}

/** How many of them came after `seen`; all of them while the tool window was never open. */
export function unreadCount(
  info: Pick<PullRequestInfo, "conversation"> | undefined,
  threads: readonly Thread[],
  seen: number | undefined,
): number {
  return othersSaid(info, threads).filter((at) => seen === undefined || at > seen).length;
}

/** The time to remember as seen when the tool window is open: now, or the newest comment if the platform's clock is ahead. */
export function seenNow(
  info: Pick<PullRequestInfo, "conversation"> | undefined,
  threads: readonly Thread[],
  now: number,
) {
  return Math.max(now, ...othersSaid(info, threads));
}

/** What the time is remembered under: the pull request, whichever clone of its repository shows it. */
export const seenKey = (key: ChangesetKey | undefined): string | undefined =>
  key?.type === "pullRequest" ? `vet.commentsSeen.${key.remote}#${key.number}` : undefined;
