// The viewer's comments and staged vote on their way to the platform, as the toolbar's outgoing
// button counts them and its popover lists them.

import type { PendingVote } from "../../generated/PendingVote";
import type { PullRequestInfo } from "../../generated/PullRequestInfo";
import type { Thread } from "../../generated/Thread";
import type { ThreadMessage } from "../../generated/ThreadMessage";
import { isSubmitKey } from "../../lib/composer.ts";
import { noun, plural } from "../../lib/format.ts";
import { platformName } from "../../lib/platformName.ts";
import type { MenuItem } from "../../lib/ui/menuItem.ts";
import { inOrder } from "./groups.ts";

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

/** The staged vote counts as one outgoing item, in the state of a comment. */
export function summarizeOutgoing(
  list: readonly OutgoingComment[],
  vote: PendingVote | null | undefined,
): OutgoingSummary {
  const states = [...list.map((o) => o.message.state.type), ...(vote ? [vote.state.type] : [])];
  return {
    count: states.length,
    failed: states.filter((s) => s === "failed").length,
    posting: states.includes("posting"),
  };
}

/** The button's tooltip: how much is outgoing, and how much of it failed. A staged vote makes them items, not comments. */
export function outgoingTitle(summary: OutgoingSummary, withVote: boolean): string {
  const what = (n: number) => noun(n, withVote ? "item" : "comment");
  if (summary.failed > 0) return `${summary.failed} of ${summary.count} outgoing ${what(summary.count)} failed to post`;
  if (summary.count > 0) return `${summary.count} outgoing ${what(summary.count)}`;
  return "No outgoing comments";
}

/** The staged vote as the popover lists it: `Vote: Approve`. */
export function voteText(info: Pick<PullRequestInfo, "traits">, vote: PendingVote): string {
  return `Vote: ${info.traits.verdicts.find((v) => v.id === vote.verdict)?.label ?? vote.verdict}`;
}

/** The platform's verdicts as the vote picker lists them, the staged vote marked, else the one the viewer has cast. */
export function voteChoices(info: Pick<PullRequestInfo, "traits" | "pendingVote" | "viewerVerdict">): MenuItem[] {
  const chosen = info.pendingVote?.verdict ?? info.viewerVerdict;
  return info.traits.verdicts.map(({ id, label }) => ({ id, label, checked: id === chosen, radio: true }));
}

/** Where posting sends the comments: `to #123 on GitHub`. */
export const destination = (info: Pick<PullRequestInfo, "number" | "platform">): string =>
  `to #${info.number} on ${platformName(info.platform.kind)}`;

/** What the Post button says it posts: the comments that wait, and the vote if one does. */
export function postLabel(comments: number, vote: boolean): string {
  if (comments === 0 && vote) return "Post vote";
  const n = `Post ${plural(comments, "comment")}`;
  return vote ? `${n} and vote` : n;
}

/** Whether posting can go ahead: something waits, nothing is on its way, and the platform lets the viewer. */
export const canPost = (waiting: number, busy: boolean, blocked: string | undefined): boolean =>
  waiting > 0 && !busy && blocked === undefined;

/**
 * What a key does in the popover's list: the submit key posts what the Post button would, even in
 * a text field, where it would otherwise add a line. The vote picker has no posting; Enter there
 * stages the vote.
 */
export function outgoingKey(
  e: Pick<KeyboardEvent, "key" | "ctrlKey" | "metaKey">,
  picking: boolean,
): "post" | undefined {
  return !picking && isSubmitKey(e) ? "post" : undefined;
}
