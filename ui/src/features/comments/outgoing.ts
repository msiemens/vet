// The viewer's comments on their way to the platform, as the toolbar's outgoing-comments button
// counts them and its popover lists them.

import type { PullRequestInfo } from "../../generated/PullRequestInfo";
import type { Thread } from "../../generated/Thread";
import type { ThreadMessage } from "../../generated/ThreadMessage";
import { platformName } from "../../lib/platformName.ts";
import { inOrder, rowTitle } from "./groups.ts";

export interface OutgoingComment {
  thread: Thread;
  message: ThreadMessage;
}

/** The comments the platform does not have yet, in the order the Comments tool window lists their threads, each thread's as written. */
export function outgoingComments(threads: readonly Thread[]): OutgoingComment[] {
  return threads
    .filter((t) => t.platform !== null)
    .sort(inOrder)
    .flatMap((thread) =>
      thread.messages.filter((m) => m.state.type !== "posted").map((message) => ({ thread, message })),
    );
}

export interface OutgoingSummary {
  count: number;
  /** How many the platform refused. */
  failed: number;
  /** Some are on their way to the platform. */
  posting: boolean;
}

export function summarizeOutgoing(list: readonly OutgoingComment[]): OutgoingSummary {
  return {
    count: list.length,
    failed: list.filter((o) => o.message.state.type === "failed").length,
    posting: list.some((o) => o.message.state.type === "posting"),
  };
}

/** Where a comment is: `job.rs:12`, `job.rs` for one on the file, or the pull request. */
export const outgoingPlace = (thread: Thread): string =>
  thread.kind === "changeset" ? "on the pull request" : rowTitle(thread);

/** Where posting sends the comments: `to #123 on GitHub`. */
export const destination = (info: Pick<PullRequestInfo, "number" | "platform">): string =>
  `to #${info.number} on ${platformName(info.platform.kind)}`;

export const postLabel = (n: number): string => `Post ${n} ${n === 1 ? "comment" : "comments"}`;
