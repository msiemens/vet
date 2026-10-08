// What the two tabs of the Comments tool window list for a pull request: Review the threads on
// files and lines, Conversation what is said about the pull request as a whole. On a platform with
// a conversation feed that is the feed with the viewer's comments that wait for Submit review; on
// one without, the threads on the pull request as a whole.

import type { ConversationEntry } from "../../generated/ConversationEntry";
import type { Thread } from "../../generated/Thread";
import type { ThreadMessage } from "../../generated/ThreadMessage";
import { isFlat } from "../../lib/flatThread.ts";

/** The threads the Review tab lists. Without a pull request there is no Conversation tab, so it lists every thread. */
export function reviewThreads(threads: readonly Thread[], pullRequest: boolean): Thread[] {
  return pullRequest ? threads.filter((t) => t.kind !== "changeset") : [...threads];
}

/** The threads on the pull request as a whole, which the Conversation tab shows where there is no feed. */
export const changesetThreads = (threads: readonly Thread[]): Thread[] => threads.filter((t) => t.kind === "changeset");

/** A comment of the viewer's on the pull request that waits for Submit review. */
export interface DraftComment {
  thread: Thread;
  message: ThreadMessage;
}

/** The viewer's comments on the pull request that the platform does not have yet, in the order they were written. */
export function draftComments(threads: readonly Thread[]): DraftComment[] {
  return threads
    .filter(isFlat)
    .flatMap((thread) =>
      thread.messages.filter((m) => m.state.type !== "posted").map((message) => ({ thread, message })),
    )
    .sort((a, b) => a.message.at - b.message.at || a.message.id - b.message.id);
}

/** The viewer wrote it, and can edit and delete it. */
export const isOwnComment = (entry: ConversationEntry) => entry.kind.type === "comment" && entry.kind.mine;

export type VerdictTone = "approved" | "changes" | "commented";

/** What a review's chip says and how it is coloured; nothing for a comment or a withdrawn review. */
export function verdictChip(entry: ConversationEntry): { label: string; tone: VerdictTone } | undefined {
  if (entry.kind.type !== "review") return undefined;
  switch (entry.kind.verdict) {
    case "approved":
      return { label: "Approved", tone: "approved" };
    case "changesRequested":
      return { label: "Changes requested", tone: "changes" };
    case "commented":
      return { label: "Commented", tone: "commented" };
    default:
      return undefined;
  }
}
