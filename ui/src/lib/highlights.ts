// Syntax highlighting as the backend sends it (`Highlights` in the protocol): the UI parses
// nothing, it only decodes the ranges and puts a class on each. Offsets are UTF-16 units of the
// document, as CodeMirror counts them.

/** The CSS class of each kind, in the protocol's numbering; the colours are in `highlights.css`. */
const CLASSES = [
  "syn-keyword",
  "syn-string",
  "syn-number",
  "syn-comment",
  "syn-type",
  "syn-function",
  "syn-attribute",
  "syn-heading",
  "syn-link",
  "syn-qualifier",
] as const;

/** Coloured ranges in order and not overlapping. */
export interface Highlights {
  readonly from: Uint32Array;
  readonly to: Uint32Array;
  readonly kind: Uint8Array;
}

export const NO_HIGHLIGHTS: Highlights = { from: new Uint32Array(0), to: new Uint32Array(0), kind: new Uint8Array(0) };

/** Decodes the wire form: a gap since the end of the previous range, a length and a kind, per range. */
export function decodeHighlights(data: readonly number[]): Highlights {
  const count = Math.floor(data.length / 3);
  if (!count) return NO_HIGHLIGHTS;
  const from = new Uint32Array(count);
  const to = new Uint32Array(count);
  const kind = new Uint8Array(count);
  let end = 0;
  for (let i = 0; i < count; i++) {
    from[i] = end + data[3 * i];
    end = to[i] = from[i] + data[3 * i + 1];
    kind[i] = data[3 * i + 2];
  }
  return { from, to, kind };
}

/** Index of the first range that ends after `pos`. */
function firstAfter(h: Highlights, pos: number): number {
  let lo = 0;
  let hi = h.to.length;
  while (lo < hi) {
    const mid = (lo + hi) >> 1;
    if (h.to[mid] > pos) hi = mid;
    else lo = mid + 1;
  }
  return lo;
}

/** Calls `put` for each range that overlaps `from..to`, cut to it, with its class; a kind this UI does not know is skipped. */
export function eachRange(
  h: Highlights,
  from: number,
  to: number,
  put: (from: number, to: number, className: string) => void,
) {
  for (let i = firstAfter(h, from); i < h.from.length && h.from[i] < to; i++) {
    const className = CLASSES[h.kind[i]] as string | undefined;
    if (className) put(Math.max(h.from[i], from), Math.min(h.to[i], to), className);
  }
}

/** A stretch of text that is plain (a string) or coloured. */
export type Piece = string | { text: string; className: string };

/** `text`, which starts at `start` in the document, cut into plain and coloured pieces. */
export function pieces(h: Highlights, start: number, text: string): Piece[] {
  const out: Piece[] = [];
  let at = 0;
  eachRange(h, start, start + text.length, (from, to, className) => {
    if (from - start > at) out.push(text.slice(at, from - start));
    out.push({ text: text.slice(from - start, to - start), className });
    at = to - start;
  });
  if (at < text.length) out.push(text.slice(at));
  return out;
}

/** The ranges after an edit: positions map through it, and a range the edit removed is dropped. */
export function mapHighlights(h: Highlights, mapPos: (pos: number, assoc: -1 | 1) => number): Highlights {
  const from: number[] = [];
  const to: number[] = [];
  const kind: number[] = [];
  for (let i = 0; i < h.from.length; i++) {
    const start = mapPos(h.from[i], 1);
    const end = mapPos(h.to[i], -1);
    if (end <= start) continue;
    from.push(start);
    to.push(end);
    kind.push(h.kind[i]);
  }
  return { from: Uint32Array.from(from), to: Uint32Array.from(to), kind: Uint8Array.from(kind) };
}
