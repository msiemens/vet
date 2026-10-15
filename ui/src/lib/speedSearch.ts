// Speed search: what typing into a focused list, tree or menu means, and how a typed query matches
// a name. Domain-free; `lib/ui/speedSearch.svelte.ts` holds the state a list keeps.

import type { KeyLike } from "./keymap.ts";

/** A half-open range of characters of a text. */
export type Range = [start: number, end: number];

export interface Match {
  /** Higher is better: names that start with the query, or match it at word starts, rank first. */
  score: number;
  /** Where the query matched in the text, merged and sorted, for highlighting. */
  ranges: Range[];
}

const HUMP = 5;
const CONSECUTIVE = 6;
const WITHIN_WORD = 2;
const FIRST = 10;
const AT_START = 4;
const SAME_CASE = 1;

const isLetter = (c: string) => /\p{L}/u.test(c);
const isDigit = (c: string) => /\p{N}/u.test(c);

// Per-code-unit classes, computed once per name: whether it is a word character and its case.
const WORD = 1;
const UPPER = 2;
const LOWER = 4;
const DIGIT = 8;

// Scratch buffers for one match, grown as needed and reused (speed search runs on one thread).
let cls = new Uint8Array(0);
let hump = new Uint8Array(0);
let codes = new Int32Array(0);
let lowered = new Int32Array(0);
let from = new Int32Array(0);
let prev = new Float64Array(0);
let cur = new Float64Array(0);
let segStart = new Int32Array(0);

function grow(n: number, m: number): void {
  if (cls.length < n) {
    cls = new Uint8Array(n);
    hump = new Uint8Array(n);
    codes = new Int32Array(n);
    lowered = new Int32Array(n);
    prev = new Float64Array(n);
    cur = new Float64Array(n);
    segStart = new Int32Array(n);
  }
  if (from.length < m * n) from = new Int32Array(m * n);
}

/** Whether two code units are equal ignoring case, as `a.toLowerCase() === b.toLowerCase()` for single ones. */
function eqLower(a: number, b: number): boolean {
  if (a < 128 && b < 128) return (a >= 65 && a <= 90 ? a + 32 : a) === (b >= 65 && b <= 90 ? b + 32 : b);
  return String.fromCharCode(a).toLowerCase() === String.fromCharCode(b).toLowerCase();
}

/** The lowercase of one code unit, or -1 when it lowercases to more than one. */
function lowerCode(code: number): number {
  if (code < 128) return code >= 65 && code <= 90 ? code + 32 : code;
  const l = String.fromCharCode(code).toLowerCase();
  return l.length === 1 ? l.charCodeAt(0) : -1;
}

/** Fills `cls`, `codes` and `lowered` with each code unit's class, its code, and its lowercase. */
function classify(text: string, n: number): void {
  for (let i = 0; i < n; i++) {
    const code = text.charCodeAt(i);
    let f = 0;
    if (code < 128) {
      if (code >= 65 && code <= 90) f = WORD | UPPER;
      else if (code >= 97 && code <= 122) f = WORD | LOWER;
      else if (code >= 48 && code <= 57) f = WORD | DIGIT;
      lowered[i] = code >= 65 && code <= 90 ? code + 32 : code;
    } else {
      const c = String.fromCharCode(code);
      const l = c.toLowerCase();
      lowered[i] = l.length === 1 ? l.charCodeAt(0) : -1;
      if (isLetter(c)) {
        f = WORD;
        if (c === c.toUpperCase() && c !== c.toLowerCase()) f |= UPPER;
        else if (c === c.toLowerCase() && c !== c.toUpperCase()) f |= LOWER;
      } else if (isDigit(c)) f = WORD | DIGIT;
    }
    codes[i] = code;
    cls[i] = f;
  }
}

/** Fills `hump` with whether each position starts a word: after a separator, at a lower-to-upper change (`ChangesetView`), at the end of an acronym (`XMLParser`), and `segStart` with where the word a position belongs to begins. */
function humps(n: number): void {
  let start = 0;
  for (let i = 0; i < n; i++) {
    const f = cls[i];
    let h = 0;
    if (f & WORD) {
      if (i === 0) h = 1;
      else {
        const b = cls[i - 1];
        if (!(b & WORD)) h = 1;
        else if (f & UPPER && (b & LOWER || b & DIGIT || (b & UPPER && i + 1 < n && cls[i + 1] & LOWER))) h = 1;
      }
    }
    hump[i] = h;
    if (h) start = i;
    segStart[i] = start;
  }
}

function toRanges(indices: number[]): Range[] {
  const ranges: Range[] = [];
  for (const i of indices) {
    const last = ranges.at(-1);
    if (last?.[1] === i) last[1] = i + 1;
    else ranges.push([i, i + 1]);
  }
  return ranges;
}

/**
 * The best way to place the query's characters, in order, on word starts and runs after them. Each
 * row of the grid is read in one pass: a run onto a word start takes the best earlier cell up to
 * there, a run within a word the best earlier cell of that word, so the work is close to the product
 * of the two lengths. `from` holds the way back for the ranges.
 */
function matchHumps(query: string, text: string, m: number, n: number): Match | undefined {
  if (m > n) return undefined;
  grow(n, m);
  classify(text, n);
  humps(n);

  const q0 = query.charCodeAt(0);
  const l0 = lowerCode(q0);
  for (let i = 0; i < n; i++) {
    cur[i] = -Infinity;
    from[i] = -1;
    if (!hump[i] || !sameCode(q0, l0, codes[i], lowered[i])) continue;
    cur[i] = FIRST + (i === 0 ? AT_START : 0) + (q0 === codes[i] ? SAME_CASE : 0);
  }

  for (let j = 1; j < m; j++) {
    // The row just read becomes `prev`; the buffers trade places, so nothing is copied.
    const done = prev;
    prev = cur;
    cur = done;
    const qj = query.charCodeAt(j);
    const lj = lowerCode(qj);
    const at = j * n;
    // The best `prev` up to the position two back, the first position that reached it: the run onto
    // a word start.
    let bv = -Infinity;
    let bk = -1;
    for (let i = 0; i < n; i++) {
      const k = i - 2;
      if (k >= 0) {
        const p = prev[k];
        if (p > bv) {
          bv = p;
          bk = k;
        }
      }
      cur[i] = -Infinity;
      from[at + i] = -1;
      if (!sameCode(qj, lj, codes[i], lowered[i])) continue;
      const exact = qj === codes[i] ? SAME_CASE : 0;
      if (i >= 2) {
        if (hump[i]) {
          if (bv > -Infinity) {
            cur[i] = bv + HUMP + exact;
            from[at + i] = bk;
          }
        } else {
          // The word's own earlier cells, in order, exactly as the plain scan reads them.
          for (let k = segStart[i]; k <= i - 2; k++) {
            const before = prev[k];
            if (before === -Infinity) continue;
            const gain = WITHIN_WORD - 0.1 * (i - k - 1);
            const v = before + gain + exact;
            if (v > cur[i]) {
              cur[i] = v;
              from[at + i] = k;
            }
          }
        }
      }
      if (i >= 1 && prev[i - 1] > -Infinity) {
        const v = prev[i - 1] + CONSECUTIVE + exact;
        if (v > cur[i]) {
          cur[i] = v;
          from[at + i] = i - 1;
        }
      }
    }
  }

  let last = -1;
  for (let i = 0; i < n; i++) if (cur[i] > -Infinity && (last < 0 || cur[i] > cur[last])) last = i;
  if (last < 0) return undefined;
  const indices: number[] = [];
  for (let j = m - 1, i = last; j >= 0; j--) {
    indices.unshift(i);
    i = from[j * n + i];
  }
  return { score: cur[last], ranges: toRanges(indices) };
}

/** Whether the query code unit `q` and the text code unit `t` are the same ignoring case. */
function sameCode(q: number, qLower: number, t: number, tLower: number): boolean {
  if (qLower >= 0 && tLower >= 0) return qLower === tLower;
  return eqLower(q, t);
}

/** A run of the text anywhere, which ranks below any match on word starts. */
function matchSubstring(queryLower: string, textLower: string, length: number): Match | undefined {
  const at = textLower.indexOf(queryLower);
  return at < 0 ? undefined : { score: length * 0.5, ranges: [[at, at + length]] };
}

/**
 * Matches `query` against `text` ignoring case, the way IntelliJ's speed search does: its
 * characters sit on the starts of words (`CSV` in `ChangesetView`, `rc` in `release_candidate`) or
 * run on from them (`chang`, `setV`), or the query is a plain run of the text. An empty query
 * matches everything.
 */
export function match(query: string, text: string): Match | undefined {
  return matchLowered(query, query.toLowerCase(), text, text.toLowerCase());
}

/**
 * `match` with the two lowercased forms already at hand, so a caller that keeps them (rank) need
 * not lowercase the same name on every keystroke. The names are not written through.
 */
export function matchLowered(query: string, queryLower: string, text: string, textLower: string): Match | undefined {
  if (!query) return { score: 0, ranges: [] };
  const humps = matchHumps(query, text, query.length, text.length);
  const run = matchSubstring(queryLower, textLower, query.length);
  const found = humps && run ? (humps.score >= run.score ? humps : run) : (humps ?? run);
  return found && { ...found, score: found.score - text.length * 0.01 };
}

/** The index of the best match of `query` among `texts` (the first of equals), or -1. */
export function bestMatch(query: string, texts: readonly string[]): number {
  let best = -1;
  let bestScore = -Infinity;
  texts.forEach((text, i) => {
    const score = match(query, text)?.score;
    if (score !== undefined && score > bestScore) [best, bestScore] = [i, score];
  });
  return best;
}

/**
 * The row a search aims at, the one Enter opens: the best match of `query` among the rows that
 * matched (`found`, indices into the rows, `nameOf` their names), or -1 for none.
 */
export function aimIndex(query: string, found: readonly number[], nameOf: (index: number) => string): number {
  return found[Math.max(0, bestMatch(query, found.map(nameOf)))] ?? -1;
}

/**
 * The matched row that Up or Down (`by` 1 or -1) steps to from `cursor`, wrapping round; from a row
 * that is none of them, the first for Down and the last for Up. None when no row matched.
 */
export function stepMatch(found: readonly number[], cursor: number, by: number): number | undefined {
  if (!found.length) return undefined;
  const at = found.indexOf(cursor);
  return found[at < 0 ? (by > 0 ? 0 : found.length - 1) : (at + by + found.length) % found.length];
}

/** The parts of `ranges` (merged, sorted) inside `[from, to)`, relative to `from`. */
export function sliceRanges(ranges: readonly Range[], from: number, to: number): Range[] {
  return ranges
    .filter(([start, end]) => start < to && end > from)
    .map(([start, end]) => [Math.max(start, from) - from, Math.min(end, to) - from]);
}

/** `text` cut into runs, each marked whether it lies in one of the (merged, sorted) `ranges`. */
export function highlightRuns(text: string, ranges: readonly Range[]): { text: string; hit: boolean }[] {
  const runs: { text: string; hit: boolean }[] = [];
  let at = 0;
  for (const [start, end] of ranges) {
    if (start > at) runs.push({ text: text.slice(at, start), hit: false });
    runs.push({ text: text.slice(start, end), hit: true });
    at = end;
  }
  if (at < text.length) runs.push({ text: text.slice(at), hit: false });
  return runs;
}

/**
 * The attribute of an element whose focus is in an active search. The command registry treats it as
 * a text field: a command without `inFields` leaves its key (a Space, say) to the search.
 */
export const SEARCHING = "data-speed-searching";

/** A keydown as speed search reads it: the key and where it landed. */
export type SpeedSearchKeyEvent = KeyLike & { target?: EventTarget | null };

const FIELD = "input, textarea, select, [contenteditable]:not([contenteditable='false'])";

/**
 * The query after a key, or undefined when the key is not the search's: typing in a text field or an
 * editor, shortcuts, and Escape or Backspace with nothing typed. A character types into the query
 * (a space only after another character, so it can still press a button); Backspace deletes one;
 * Escape clears.
 */
export function speedSearchKey(query: string, e: SpeedSearchKeyEvent): string | undefined {
  const target = e.target as { closest?: (selector: string) => unknown } | null | undefined;
  if (target?.closest?.(FIELD) != null) return undefined;
  if (e.isComposing || e.keyCode === 229) return undefined;
  if (e.key === "Escape") return query ? "" : undefined;
  if (e.key === "Backspace") return query ? Array.from(query).slice(0, -1).join("") : undefined;
  // Ctrl with Alt is AltGr on Windows, which types characters.
  if (e.metaKey || (e.ctrlKey && !e.altKey)) return undefined;
  if (Array.from(e.key).length !== 1) return undefined;
  if (e.key === " " && !query) return undefined;
  return query + e.key;
}
