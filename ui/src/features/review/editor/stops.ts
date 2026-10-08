// The stops of hunk navigation: every change card and every hunk of every file, numbered over
// the whole view. A file's stops are its cards first, then its hunks.

import type { Hunk } from "../../../generated/Hunk";
import type { Side } from "../../../generated/Side";
import type { TextSpan } from "../../../generated/TextSpan";

export interface FileStops {
  cards: number;
  hunks: number;
}

export class Stops {
  /** The number of stops before each file. */
  private starts: number[] = [];
  private counts: number[];
  readonly total: number;

  constructor(files: FileStops[]) {
    let total = 0;
    this.counts = files.map((f) => f.cards + f.hunks);
    for (const count of this.counts) {
      this.starts.push(total);
      total += count;
    }
    this.total = total;
  }

  /** The stop of file `fi`'s first card, else its first hunk. */
  start(fi: number): number {
    return this.starts[fi];
  }

  /** The file of a stop, and the stop's index among that file's stops. */
  locate(stop: number): [file: number, index: number] {
    let [lo, hi] = [0, this.counts.length - 1];
    while (lo < hi) {
      const mid = (lo + hi) >> 1;
      if (this.starts[mid] + this.counts[mid] > stop) hi = mid;
      else lo = mid + 1;
    }
    return [lo, stop - this.starts[lo]];
  }

  clamp(stop: number): number {
    return Math.max(0, Math.min(this.total - 1, stop));
  }

  /**
   * The stop of file `fi` nearest to where another view of the file stood: the card at index
   * `card` (-1: none), else the hunk starting nearest to `line`, else the file's first stop.
   * `starts` are the hunks' target start lines and `cards` the file's number of cards.
   */
  nearest(fi: number, cards: number, card: number, starts: number[], line: number): number | undefined {
    if (card >= 0) return this.starts[fi] + card;
    const hunk = nearestHunk(starts, line);
    if (hunk >= 0) return this.starts[fi] + cards + hunk;
    return cards ? this.starts[fi] : undefined;
  }
}

/** The index of the last hunk starting at or above `line` (the first hunk above all of them). */
export function hunkAt(starts: number[], line: number): number {
  let hunk = 0;
  starts.forEach((start, i) => {
    if (start <= line) hunk = i;
  });
  return hunk;
}

/** The index of the first stop at or below `y` among stops in order, else the last one; -1 without stops. */
export function firstStopFrom(tops: readonly number[], y: number): number {
  const at = tops.findIndex((top) => top >= y);
  return at < 0 ? tops.length - 1 : at;
}

/**
 * The index of the first hunk whose lines on `side` meet the 0-based `span`; -1 if none does. A hunk
 * with no lines on that side (a pure insertion seen from the base) sits at its start.
 */
export function hunkOverlapping(hunks: readonly Hunk[], side: Side, span: TextSpan): number {
  const first = span.start.line;
  // The end is exclusive: an end at column 0 belongs to the line before.
  const end = Math.max(first + 1, span.end.col === 0 ? span.end.line : span.end.line + 1);
  return hunks.findIndex((h) => {
    const start = side === "base" ? h.baseStart : h.targetStart;
    const stop = start + Math.max(1, side === "base" ? h.baseLen : h.targetLen);
    return start < end && first < stop;
  });
}

/** The index of the hunk whose start is nearest to `line`, the first of equals; -1 without hunks. */
export function nearestHunk(starts: number[], line: number): number {
  let nearest = -1;
  starts.forEach((start, i) => {
    if (nearest < 0 || Math.abs(start - line) < Math.abs(starts[nearest] - line)) nearest = i;
  });
  return nearest;
}
