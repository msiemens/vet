// Where a file's comment threads show and how tall their cards are expected to be, from
// data alone. A range thread's card sits below the target line the backend names. A card's
// real height is measured once it is drawn (the text wraps), so the estimate only has to be close.

import type { Thread } from "../../../generated/Thread";
import type { ThreadTarget } from "../../../generated/ThreadTarget";

/** What a card shows: a thread, or the form that opens one. */
export interface CardData {
  thread?: Thread;
  draft?: ThreadTarget;
  /** A resolved thread the viewer opened up; it shows as one row otherwise. */
  open?: boolean;
}

/** Identifies a card among those of a view, whatever its content. */
export const cardKey = (card: CardData): string => (card.thread ? `t${card.thread.id}` : "draft");

/** Identifies a card's shape: what its measured height is good for when the same card is estimated again. */
export const heightKey = (card: CardData): string =>
  card.thread
    ? `${cardKey(card)}/${card.thread.status}/${card.thread.messages.length}/${card.open ? "open" : "folded"}/${card.thread.anchor?.type}`
    : "draft";

/** A card shown below target line `line` (0-based). */
export interface InlineCard extends CardData {
  line: number;
}

export interface Placement {
  /** Threads of the whole file, above its first hunk. */
  file: Thread[];
  /** Threads of a line range, in text order. */
  inline: (InlineCard & { thread: Thread })[];
}

/**
 * Splits the threads of a file: its own, and the ones on lines, each at the line the backend
 * says its card sits below. A detached thread has neither a location nor a line: it is shown in
 * the Comments tool window only.
 */
export function placeThreads(threads: readonly Thread[]): Placement {
  const placement: Placement = { file: [], inline: [] };
  for (const thread of threads) {
    if (thread.kind === "file") placement.file.push(thread);
    else if (thread.kind === "range" && thread.targetLine !== null) {
      placement.inline.push({ thread, line: thread.targetLine });
    }
  }
  placement.inline.sort((a, b) => a.line - b.line || a.thread.id - b.thread.id);
  return placement;
}

/** Row height and the rows a card is made of; the card's own paddings are in `threadCard.ts`. */
export interface CardMetrics {
  line: number;
}

const CARD_PAD = 14;
/** The reply row: a one-row field and its buttons. */
const REPLY = 34;
/** A new comment's form: a field of three rows and the buttons below. */
const FORM = 92;
/** Characters a message line holds before it wraps, in a card of ordinary width. */
const WRAP = 100;

/** Rows a message body takes, wrapped at about `WRAP` characters. */
export function bodyRows(body: string): number {
  return body.split("\n").reduce((rows, text) => rows + Math.max(1, Math.ceil(text.length / WRAP)), 0);
}

/** A resolved thread shows as one row until it is opened. */
export function cardHeight(card: CardData, m: CardMetrics): number {
  const { thread } = card;
  if (!thread) return CARD_PAD + m.line + FORM;
  if (thread.status === "resolved" && !card.open) return CARD_PAD + m.line;
  const messages = thread.messages.reduce((h, msg) => h + m.line + bodyRows(msg.body) * m.line, 0);
  return CARD_PAD + m.line + messages + REPLY;
}
