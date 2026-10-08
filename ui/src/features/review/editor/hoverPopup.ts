// When a hover popup appears, stays, is replaced and closes (SPEC §7), apart from the DOM it is
// drawn in. A popup belongs to a hover range (the symbol under the pointer, named by a key): it is
// neither rebuilt nor moved while the pointer stays on that range, and the pointer may cross over
// into the popup within a grace period. Only a different symbol replaces it.

import { isAbort } from "../../../lib/abort.ts";

/** How long the pointer rests on a symbol before the hover query is sent. */
export const HOVER_MS = 50;

/** How long a popup outlives the pointer leaving its symbol, which lets the pointer reach the popup. */
export const GRACE_MS = 300;

/** The first and last UTF-16 columns of the word in `text` that column `col` is in; a lone punctuation mark is a word; blank is none. */
export function wordRange(text: string, col: number): { from: number; to: number } | undefined {
  if (col < 0 || col >= text.length || /\s/.test(text[col])) return undefined;
  const isWord = (i: number) => i >= 0 && i < text.length && /[\p{L}\p{N}_$]/u.test(text[i]);
  if (!isWord(col)) return { from: col, to: col + 1 };
  let from = col;
  let to = col + 1;
  while (isWord(from - 1)) from--;
  while (isWord(to)) to++;
  return { from, to };
}

export interface HoverKeyed {
  /** Identifies the hover range: the same key is the same symbol. */
  key: string;
}

export interface HoverHost<T extends HoverKeyed, C> {
  /**
   * What builds the popup for a target, or nothing if there is nothing to show. `signal` aborts
   * when the answer is no longer wanted: the pointer moved on, or the popup closed.
   */
  query(target: T, signal: AbortSignal): Promise<C | undefined>;
  /** Draws the popup and returns what removes it. */
  open(target: T, content: C): () => void;
}

export class HoverPopup<T extends HoverKeyed, C> {
  private shown?: { key: string; close: () => void };
  private pending?: { key: string; timer: ReturnType<typeof setTimeout> };
  private grace?: ReturnType<typeof setTimeout>;
  /** Identifies the latest wish for a popup, so a slow answer to an earlier one is dropped. */
  private seq = 0;
  /** The query underway, if any. */
  private asking?: AbortController;
  /** A symbol whose popup was dismissed, which stays closed until the pointer has left the symbol. */
  private dismissed?: string;

  private readonly host: HoverHost<T, C>;

  constructor(host: HoverHost<T, C>) {
    this.host = host;
  }

  get isOpen(): boolean {
    return this.shown !== undefined;
  }

  /**
   * The pointer moved. `target` is the symbol under it, if any; `overPopup` says it is on the popup,
   * which then stays whatever else is under the pointer.
   */
  pointer(target: T | undefined, overPopup = false) {
    if (overPopup) {
      this.cancelPending();
      this.cancelGrace();
      return;
    }
    if (target?.key !== this.dismissed) this.dismissed = undefined;
    if (!target) {
      this.cancelPending();
      this.armGrace();
    } else if (target.key === this.shown?.key) {
      this.cancelPending();
      this.cancelGrace();
    } else if (target.key === this.dismissed) {
      this.cancelPending();
      this.armGrace();
    } else if (target.key !== this.pending?.key) {
      this.cancelPending();
      this.armGrace();
      this.pending = { key: target.key, timer: setTimeout(() => void this.fetch(target), HOVER_MS) };
    }
  }

  /** Closes the popup for a reason other than the pointer's whereabouts (Escape, a scroll, a change), and keeps it closed on its symbol. */
  dismiss() {
    this.dismissed = this.shown?.key ?? this.pending?.key ?? this.dismissed;
    this.close();
  }

  /** Closes the popup and drops what is on its way. */
  close() {
    this.cancelPending();
    this.cancelGrace();
    this.shown?.close();
    this.shown = undefined;
  }

  private async fetch(target: T) {
    const seq = this.seq;
    const asking = new AbortController();
    this.asking = asking;
    let content: C | undefined;
    try {
      content = await this.host.query(target, asking.signal);
    } catch (error) {
      if (!isAbort(error)) throw error;
    } finally {
      if (this.asking === asking) this.asking = undefined;
      if (seq === this.seq) this.pending = undefined;
    }
    if (seq !== this.seq || content === undefined) return;
    this.shown?.close();
    this.cancelGrace();
    this.shown = { key: target.key, close: this.host.open(target, content) };
  }

  private cancelPending() {
    this.seq++;
    this.asking?.abort();
    this.asking = undefined;
    if (this.pending) clearTimeout(this.pending.timer);
    this.pending = undefined;
  }

  private armGrace() {
    if (!this.shown || this.grace !== undefined) return;
    this.grace = setTimeout(() => {
      this.grace = undefined;
      this.close();
    }, GRACE_MS);
  }

  private cancelGrace() {
    clearTimeout(this.grace);
    this.grace = undefined;
  }
}
