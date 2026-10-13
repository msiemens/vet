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
  nearest,
  step,
} from "./findMatches.ts";
import { type FindSummary, summarize } from "./findView.ts";

/** Texts over this many characters (both sides) are not searched. */
export const MAX_SEARCH_CHARS = 4_000_000;

export class FindRun {
  readonly options: FindOptions;
  readonly sets: (readonly FindMatch[] | undefined)[];
  private mode: FindMode;
  private re?: RegExp;
  private skipped = 0;
  private error?: string;
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
    const compiled = compile(options);
    if (compiled.type === "ok") this.re = compiled.re;
    else {
      if (compiled.type === "error") this.error = compiled.message;
      this.sets.fill([]);
    }
  }

  /** Whether the query can find anything: it is not empty, and the engine takes it. */
  get active(): boolean {
    return this.re !== undefined;
  }

  /** The matches of a section whose text arrived (or `undefined` for one too large to search). */
  add(section: number, file: FileText | undefined) {
    if (!this.re) return;
    if (!file || file.base.length + file.target.length > MAX_SEARCH_CHARS) {
      this.skipped++;
      this.sets[section] = [];
    } else {
      this.sets[section] = findInFile(file, this.re, this.options, this.mode);
    }
    this.settle();
  }

  /** The current match is a match: the first at or after where it was, else the first of all. */
  private settle() {
    const sets: SectionMatches = this.sets;
    const here = this.current;
    if (here && sets[here.section]?.some((m) => m.order === here.order && m.from === here.from)) return;
    this.current = nearest(sets, here ?? { section: 0, order: -1, from: 0 }) ?? here;
  }

  summary(): FindSummary {
    const s = summarize(this.sets, this.current, this.skipped);
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
