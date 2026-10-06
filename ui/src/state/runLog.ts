// The output of one run as the UI holds it: decoded text in a bounded scrollback. Not
// reactive; a view reads it with `since` when `onchange` tells it to.

import type { RunStream } from "../generated/RunStream";

export interface Piece {
  seq: number;
  stream: RunStream;
  text: string;
}

/** Characters kept; older pieces are dropped first. */
export const MAX_CHARS = 1_000_000;
/** Pieces kept, so a run of tiny writes cannot grow the list without bound. */
export const MAX_PIECES = 20_000;

export class RunLog {
  /** Called after output was added; at most once per `push`. */
  onchange?: () => void;
  /** Output was dropped from the start to stay within the limits. */
  trimmed = false;

  private pieces: Piece[] = [];
  private head = 0;
  private chars = 0;
  private next = 0;
  private decoders: Record<RunStream, TextDecoder> = {
    stdout: new TextDecoder("utf-8", { fatal: false }),
    stderr: new TextDecoder("utf-8", { fatal: false }),
  };

  /** The sequence number the next piece gets. */
  get end(): number {
    return this.next;
  }

  /** The sequence number of the oldest piece still kept. */
  get first(): number {
    return this.pieces[this.head]?.seq ?? this.next;
  }

  /** `bytes` is base64; a multi-byte character split between two chunks decodes whole. */
  push(stream: RunStream, bytes: string) {
    const raw = Uint8Array.from(atob(bytes), (c) => c.charCodeAt(0));
    this.add(stream, this.decoders[stream].decode(raw, { stream: true }));
  }

  /** The process ended: an incomplete trailing character becomes U+FFFD. */
  finish() {
    for (const stream of ["stdout", "stderr"] as const) this.add(stream, this.decoders[stream].decode());
  }

  /** The pieces with `seq >= from` that are still kept. */
  since(from: number): Piece[] {
    return this.pieces.slice(this.head + Math.max(0, from - this.first));
  }

  private add(stream: RunStream, text: string) {
    if (!text) return;
    this.pieces.push({ seq: this.next++, stream, text });
    this.chars += text.length;
    while (
      this.head < this.pieces.length - 1 &&
      (this.chars > MAX_CHARS || this.pieces.length - this.head > MAX_PIECES)
    ) {
      this.chars -= this.pieces[this.head++].text.length;
      this.trimmed = true;
    }
    if (this.head > 1024 && this.head * 2 > this.pieces.length) {
      this.pieces = this.pieces.slice(this.head);
      this.head = 0;
    }
    this.onchange?.();
  }
}
