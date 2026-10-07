// The stops of hunk navigation: every change card and every hunk of every file, numbered over
// the whole view. A file's stops are its cards first, then its hunks.

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

/** The index of the hunk whose start is nearest to `line`, the first of equals; -1 without hunks. */
export function nearestHunk(starts: number[], line: number): number {
  let nearest = -1;
  starts.forEach((start, i) => {
    if (nearest < 0 || Math.abs(start - line) < Math.abs(starts[nearest] - line)) nearest = i;
  });
  return nearest;
}
