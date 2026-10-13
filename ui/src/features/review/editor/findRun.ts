// One find in one view: the query, the matches of each section found so far and the current match.
// A view fills it section by section as the texts arrive and draws what it holds.

import { compile } from "../../../lib/search/query.ts";
import {
  type FileText,
  type FindMatch,
  type FindMode,
  type FindOptions,
  type FindPos,
  type SectionMatches,
  findInFile,
  isMatch,
  nearest,
  nearestAfter,
  step,
} from "./findMatches.ts";
import { type Counts, type FindSummary, summarize } from "./findView.ts";

/** Texts over this many characters (both sides) are not searched. */
export const MAX_SEARCH_CHARS = 4_000_000;

export class FindRun {
  readonly options: FindOptions;
  readonly sets: (readonly FindMatch[] | undefined)[];
  private mode: FindMode;
  private re?: RegExp;
  private skipped = 0;
  private error?: string;
  private counts: Counts = { total: 0, unchanged: 0, pending: 0 };
  /** Where the current match is, which may be a place that is no match (yet). */
  current?: FindPos;

  constructor(
    options: FindOptions,
    sections: number,
    mode: FindMode,
    /** Where the current match was before, or where the search starts from. */
    seed?: FindPos,
  ) {
    this.options = options;
    this.mode = mode;
    this.sets = new Array<readonly FindMatch[] | undefined>(sections).fill(undefined);
    this.current = seed;
    this.counts.pending = sections;
    const compiled = compile(options);
    if (compiled.type === "ok") this.re = compiled.re;
    else {
      if (compiled.type === "error") this.error = compiled.message;
      this.sets.fill([]);
      this.counts.pending = 0;
    }
  }

  /** Whether the query can find anything: it is not empty, and the engine takes it. */
  get active(): boolean {
    return this.re !== undefined;
  }

  /** The matches of a section whose text arrived (or `undefined` for one too large to search). */
  add(section: number, file: FileText | undefined) {
    if (!this.re) return;
    const before = this.sets[section];
    if (before) this.count(before, -1);
    else this.counts.pending--;
    let found: readonly FindMatch[];
    if (!file || file.base.length + file.target.length > MAX_SEARCH_CHARS) {
      this.skipped++;
      found = [];
    } else {
      found = findInFile(file, this.re, this.options, this.mode);
    }
    this.sets[section] = found;
    this.count(found, 1);
    this.settle();
  }

  /** Adds (1) or takes back (-1) a section's matches to the counts. */
  private count(set: readonly FindMatch[], sign: 1 | -1) {
    this.counts.total += sign * set.length;
    for (const m of set) if (m.kind === "context") this.counts.unchanged += sign;
  }

  /** The current match is a match: the first at or after where it was, else the first of all. */
  private settle() {
    const sets: SectionMatches = this.sets;
    const here = this.current;
    // Where it was is waiting to be searched: it moves once that section has been.
    if (here && here.section < sets.length && sets[here.section] === undefined) return;
    if (here && isMatch(sets, here)) return;
    const from = here ?? { section: 0, order: -1, from: 0 };
    const after = nearestAfter(sets, from);
    if (after) {
      this.current = after;
      return;
    }
    // Nothing lies after it in the sections searched, but the sections below it may still have
    // something: the match above the start is only the one to take when they have not.
    if (here && sets.some((set, section) => set === undefined && section > here.section)) return;
    this.current = nearest(sets, from) ?? here;
  }

  summary(): FindSummary {
    const s = summarize(this.sets, this.current, this.skipped, this.counts);
    return this.error === undefined ? s : { ...s, error: this.error };
  }

  /** The match `delta` from the current one and whether that wrapped; the current match stays until `moveTo`. */
  stepFrom(delta: 1 | -1) {
    return step(this.sets, this.current, delta);
  }

  moveTo(pos: FindPos) {
    this.current = pos;
  }

  matchesOf(section: number): readonly FindMatch[] {
    return this.sets[section] ?? [];
  }
}
