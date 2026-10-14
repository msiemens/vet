// What counts as unread, everywhere: the Comments tool's rail icon and the queue panel's thread cards.

import type { ChangesetKey } from "../../generated/ChangesetKey";
import type { PullRequestInfo } from "../../generated/PullRequestInfo";
import type { Thread } from "../../generated/Thread";

/**
 * Whether the viewer has not read a comment: someone else's, written after `seen` (unix seconds), the time
 * they last looked. Nothing is unread while there is no such time.
 */
export function isUnread(comment: { mine: boolean; at: number }, seen: number | undefined): boolean {
  return seen !== undefined && !comment.mine && comment.at > seen;
}

/** Each comment on the pull request as a whole: its conversation entries (comments and reviews), and the messages of its threads. */
function onTheWhole(
  info: Pick<PullRequestInfo, "conversation"> | undefined,
  threads: readonly Thread[],
): { mine: boolean; at: number }[] {
  const conversation = (info?.conversation ?? []).map((e) => ({ mine: e.kind.mine, at: e.at }));
  const inThreads = threads.filter((t) => t.kind === "changeset").flatMap((t) => t.messages);
  return [...conversation, ...inThreads];
}

/** How many comments on the pull request as a whole are unread. Comments on lines show at their lines and do not count. */
export function unreadCount(
  info: Pick<PullRequestInfo, "conversation"> | undefined,
  threads: readonly Thread[],
  seen: number | undefined,
): number {
  return onTheWhole(info, threads).filter((c) => isUnread(c, seen)).length;
}

/** The time to remember as seen when the tool window is open: now, or the newest comment if the platform's clock is ahead. */
export function seenNow(
  info: Pick<PullRequestInfo, "conversation"> | undefined,
  threads: readonly Thread[],
  now: number,
) {
  return Math.max(
    now,
    ...onTheWhole(info, threads)
      .filter((c) => !c.mine)
      .map((c) => c.at),
  );
}

/** What the time is remembered under: the pull request, whichever clone of its repository shows it. */
export const seenKey = (key: ChangesetKey | undefined): string | undefined =>
  key?.type === "pullRequest" ? `${key.remote}#${key.number}` : undefined;
