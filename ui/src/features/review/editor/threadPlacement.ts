// Where a file's comment threads show and how tall their cards are expected to be, from
// data alone. A range thread's card sits below the last line of its range in the target
// text; a thread on a base line is mapped to the target line it faces. A card's real height
// is measured once it is drawn (the text wraps), so the estimate only has to be close.

import type { Hunk } from "../../../generated/Hunk";
import { lineRange } from "../../../lib/lineRange.ts";
import type { Side } from "../../../generated/Side";
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

/** A thread whose anchor has no place: shown in the Comments tool window only. */
export function isDetached(thread: Thread): boolean {
  return thread.anchor?.type === "stale" || thread.anchor?.type === "ambiguous";
}

/**
 * The target line that line `line` of `side` faces: itself on the target side; on the base side
 * the last target line of the hunk it was deleted or replaced in, else the line it shifted to.
 */
export function targetLine(
  hunks: Pick<Hunk, "baseStart" | "baseLen" | "targetStart" | "targetLen">[],
  side: Side,
  line: number,
): number {
  if (side === "target") return line;
  let shift = 0;
  for (const h of hunks) {
    if (line < h.baseStart) break;
    if (line < h.baseStart + h.baseLen) return Math.max(0, h.targetStart + Math.max(h.targetLen, 1) - 1);
    shift += h.targetLen - h.baseLen;
  }
  return Math.max(0, line + shift);
}

/** Splits the threads shown in a file: its own, and the ones on lines. */
export function placeThreads(
  path: string,
  hunks: Pick<Hunk, "baseStart" | "baseLen" | "targetStart" | "targetLen">[],
  targetLines: number,
  threads: readonly Thread[],
): Placement {
  const placement: Placement = { file: [], inline: [] };
  for (const thread of threads) {
    if ((thread.path ?? thread.location?.path) !== path || isDetached(thread)) continue;
    if (thread.kind === "file") placement.file.push(thread);
    else if (thread.kind === "range" && thread.location) {
      const end = thread.location.start + Math.max(thread.location.len, 1) - 1;
      const line = Math.min(targetLine(hunks, thread.side ?? "target", end), Math.max(0, targetLines - 1));
      placement.inline.push({ thread, line });
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

/** The tooltip of a "moved" badge: how the anchor was found and where it was recorded. */
export function movedTitle(thread: Thread): string {
  const anchor = thread.anchor;
  if (anchor?.type !== "inferred") return "";
  const how = anchor.strategy === "crossFileMove" ? "found in another file" : "found by its surrounding lines";
  const was = thread.original
    ? `; was ${thread.original.path}:${lineRange(thread.original.start, thread.original.len)}`
    : "";
  return `Moved: ${how}, ${anchor.confidence} confidence${was}`;
}

/** What a new comment is attached to, for the form's title. */
export function draftTitle(target: ThreadTarget): string {
  if (target.type === "changeset") return "Comment on the changeset";
  if (target.type === "file") return "Comment on this file";
  const lines = lineRange(target.start, target.len);
  return target.side === "base" ? `Comment on base ${lines}` : `Comment on ${lines}`;
}

/** The text range a selection covers, as whole lines: a selection ending at a line start does not include that line. */
export function selectedLines(
  startLine: number,
  endLine: number,
  endsAtLineStart: boolean,
): { start: number; len: number } {
  const last = endsAtLineStart && endLine > startLine ? endLine - 1 : endLine;
  return { start: startLine, len: last - startLine + 1 };
}
