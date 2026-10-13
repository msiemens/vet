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
const isUpper = (c: string) => isLetter(c) && c === c.toUpperCase() && c !== c.toLowerCase();
const isLower = (c: string) => isLetter(c) && c === c.toLowerCase() && c !== c.toUpperCase();

/** Which characters start a word of `text`: after a separator, at a lower-to-upper change (`ChangesetView`), at the end of an acronym (`XMLParser`). */
function humpStarts(text: string): boolean[] {
  return text.split("").map((c, i) => {
    if (!isLetter(c) && !isDigit(c)) return false;
    if (i === 0) return true;
    const before = text.charAt(i - 1);
    if (!isLetter(before) && !isDigit(before)) return true;
    if (!isUpper(c)) return false;
    return isLower(before) || isDigit(before) || (isUpper(before) && isLower(text.charAt(i + 1)));
  });
}

const same = (a: string, b: string) => a.toLowerCase() === b.toLowerCase();

function toRanges(indices: number[]): Range[] {
  const ranges: Range[] = [];
  for (const i of indices) {
    const last = ranges.at(-1);
    if (last?.[1] === i) last[1] = i + 1;
    else ranges.push([i, i + 1]);
  }
  return ranges;
}

/** The best way to place the query's characters, in order, on word starts and runs after them. */
function matchHumps(query: string, text: string): Match | undefined {
  const m = query.length;
  const n = text.length;
  if (m > n) return undefined;
  const hump = humpStarts(text);
  // Humps up to and including each position, to tell whether a word starts between two matches.
  const humpsUpTo: number[] = [];
  let count = 0;
  for (const h of hump) humpsUpTo.push((count += h ? 1 : 0));

  // best[j][i]: the best score with query[j] on text[i], and which position query[j - 1] had.
  const best: { score: number; from: number }[][] = [];
  for (let j = 0; j < m; j++) {
    const row: { score: number; from: number }[] = Array.from({ length: n }, () => ({ score: -Infinity, from: -1 }));
    for (let i = 0; i < n; i++) {
      const c = text.charAt(i);
      if (!same(query.charAt(j), c)) continue;
      const exact = query.charAt(j) === c ? SAME_CASE : 0;
      if (j === 0) {
        if (hump[i]) row[i].score = FIRST + (i === 0 ? AT_START : 0) + exact;
        continue;
      }
      for (let k = 0; k < i; k++) {
        const before = best[j - 1][k].score;
        if (before === -Infinity) continue;
        let gain: number;
        if (k === i - 1) gain = CONSECUTIVE;
        else if (hump[i]) gain = HUMP;
        else if (humpsUpTo[i] === humpsUpTo[k]) gain = WITHIN_WORD - 0.1 * (i - k - 1);
        else continue;
        if (before + gain + exact > row[i].score) row[i] = { score: before + gain + exact, from: k };
      }
    }
    best.push(row);
  }

  const last = best[m - 1];
  let at = -1;
  for (let i = 0; i < n; i++) if (last[i].score > -Infinity && (at < 0 || last[i].score > last[at].score)) at = i;
  if (at < 0) return undefined;
  const indices: number[] = [];
  for (let j = m - 1, i = at; j >= 0; j--) {
    indices.unshift(i);
    i = best[j][i].from;
  }
  return { score: last[at].score, ranges: toRanges(indices) };
}

/** A run of the text anywhere, which ranks below any match on word starts. */
function matchSubstring(query: string, text: string): Match | undefined {
  const at = text.toLowerCase().indexOf(query.toLowerCase());
  return at < 0 ? undefined : { score: query.length * 0.5, ranges: [[at, at + query.length]] };
}

/**
 * Matches `query` against `text` ignoring case, the way IntelliJ's speed search does: its
 * characters sit on the starts of words (`CSV` in `ChangesetView`, `rc` in `release_candidate`) or
 * run on from them (`chang`, `setV`), or the query is a plain run of the text. An empty query
 * matches everything.
 */
export function match(query: string, text: string): Match | undefined {
  if (!query) return { score: 0, ranges: [] };
  const humps = matchHumps(query, text);
  const run = matchSubstring(query, text);
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
