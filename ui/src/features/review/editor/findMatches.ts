// What a find looks for in one file, purely: the matches of a query in a file's base and target
// text, in the order a unified diff shows them (context line, then a hunk's removed lines, then its
// added ones). The search never asks an editor: the unified diff draws removed lines outside its
// document, and an unmounted section has no editor, so it reads the texts the views are given.

import type { Hunk } from "../../../generated/Hunk";
import { type Hit, lineHits } from "../../../lib/search/query.ts";

/** The sides of a diff to search; a context line is on both. */
export type FindSide = "both" | "target" | "base";

export interface FindOptions {
  text: string;
  matchCase: boolean;
  wholeWord: boolean;
  regex: boolean;
  side: FindSide;
  /** Only the lines the change added or removed. */
  onlyChanges: boolean;
}

/** How the file is shown: a diff has both sides, the File view the target's text alone. */
export type FindMode = "diff" | "file";

export interface FileText {
  base: string;
  target: string;
  hunks: readonly Hunk[];
}

/** An unchanged line (both sides), an added one (target) or a removed one (base). */
export type MatchKind = "context" | "added" | "removed";

export interface FindMatch extends Hit {
  kind: MatchKind;
  /** The 0-based line: of the target for a context or added line, of the base for a removed one. */
  line: number;
  /** The 0-based base line of a context or removed line; -1 for an added one. */
  baseLine: number;
  /** The hunk of an added or removed line; -1 for a context line. */
  hunk: number;
  /** The position in the unified diff's order, which stays when the query changes. */
  order: number;
}

const LINE_BREAK = /\r\n?|\n/;

/** Whether a line of this kind is looked at under the options. */
export function searched(kind: MatchKind, o: Pick<FindOptions, "side" | "onlyChanges">, mode: FindMode): boolean {
  if (o.onlyChanges && kind === "context") return false;
  if (mode === "file") return kind !== "removed";
  if (o.side === "target") return kind !== "removed";
  if (o.side === "base") return kind !== "added";
  return true;
}

/** The matches of `re` in the file, in the unified diff's order. */
export function findInFile(file: FileText, re: RegExp, options: FindOptions, mode: FindMode): FindMatch[] {
  const out: FindMatch[] = [];
  const target = file.target.split(LINE_BREAK);
  const base = mode === "file" ? [] : file.base.split(LINE_BREAK);
  let order = 0;
  const visit = (kind: MatchKind, text: string | undefined, line: number, baseLine: number, hunk: number) => {
    const at = order++;
    if (text === undefined || !searched(kind, options, mode)) return;
    for (const hit of lineHits(re, text)) out.push({ ...hit, kind, line, baseLine, hunk, order: at });
  };
  let t = 0;
  let b = 0;
  const context = (until: number) => {
    for (; t < until; t++, b++) visit("context", target[t], t, b, -1);
  };
  file.hunks.forEach((h, i) => {
    context(h.targetStart);
    b = h.baseStart;
    if (mode === "diff") for (let j = 0; j < h.baseLen; j++) visit("removed", base[b + j], b + j, b + j, i);
    for (let j = 0; j < h.targetLen; j++) visit("added", target[h.targetStart + j], h.targetStart + j, -1, i);
    b = h.baseStart + h.baseLen;
    t = h.targetStart + h.targetLen;
  });
  context(target.length);
  return out;
}

/** The `order` of the line of `kind` at `line` (0-based, in its side's text), as `findInFile` numbers the lines it visits. */
export function orderOf(hunks: readonly Hunk[], kind: MatchKind, line: number, mode: FindMode): number {
  let order = 0;
  let t = 0;
  for (const h of hunks) {
    if (kind === "context" && line < h.targetStart) return order + (line - t);
    order += h.targetStart - t;
    if (mode === "diff") {
      if (kind === "removed" && line >= h.baseStart && line < h.baseStart + h.baseLen)
        return order + (line - h.baseStart);
      order += h.baseLen;
    }
    if (kind === "added" && line >= h.targetStart && line < h.targetStart + h.targetLen)
      return order + (line - h.targetStart);
    order += h.targetLen;
    t = h.targetStart + h.targetLen;
  }
  return order + (line - t);
}

/** Where a match is, which holds when the query or the texts change: a section and a place in it. */
export interface FindPos {
  section: number;
  order: number;
  from: number;
}

export const posOf = (section: number, m: FindMatch): FindPos => ({ section, order: m.order, from: m.from });

const compare = (a: FindPos, b: FindPos) => a.section - b.section || a.order - b.order || a.from - b.from;

/** The matches of each section; a section not searched yet is `undefined`. */
export type SectionMatches = readonly (readonly FindMatch[] | undefined)[];

export function totalOf(sets: SectionMatches): number {
  return sets.reduce((sum, set) => sum + (set?.length ?? 0), 0);
}

/** How many matches lie before `pos`, counting the sections searched so far. */
export function indexOf(sets: SectionMatches, pos: FindPos): number {
  let n = 0;
  for (let section = 0; section < Math.min(pos.section, sets.length); section++) n += sets[section]?.length ?? 0;
  // The matches of a section are in the order of the search.
  for (const m of sets[pos.section] ?? []) {
    if (m.order > pos.order || (m.order === pos.order && m.from >= pos.from)) break;
    n++;
  }
  return n;
}

/** Whether `pos` is one of the matches. */
export function isMatch(sets: SectionMatches, pos: FindPos): boolean {
  return sets[pos.section]?.some((m) => m.order === pos.order && m.from === pos.from) ?? false;
}

/** The first match at or after `pos`; none when no match lies after it in the sections searched. */
export function nearestAfter(sets: SectionMatches, pos: FindPos): FindPos | undefined {
  for (const [section, set] of sets.entries()) {
    for (const m of set ?? []) {
      const at = posOf(section, m);
      if (compare(at, pos) >= 0) return at;
    }
  }
  return undefined;
}

/** The first match at or after `pos`, else the first of all; none without matches. */
export function nearest(sets: SectionMatches, pos: FindPos): FindPos | undefined {
  const first = sets.flatMap((set, section) => (set ?? []).slice(0, 1).map((m) => posOf(section, m)))[0];
  return nearestAfter(sets, pos) ?? first;
}

/** A place in a view of several files that survives the files before it coming and going. */
export interface PathPos {
  path: string;
  order: number;
  from: number;
}

/** The place `pos` is, named by the file; `paths` are those of the sections, in order. */
export function toPathPos(paths: readonly string[], pos: FindPos): PathPos | undefined {
  const path = paths.at(pos.section);
  return path === undefined ? undefined : { path, order: pos.order, from: pos.from };
}

/** Where `at` is in the sections now; none if its file is not among them any more. */
export function fromPathPos(paths: readonly string[], at: PathPos): FindPos | undefined {
  const section = paths.indexOf(at.path);
  return section < 0 ? undefined : { section, order: at.order, from: at.from };
}

/** The match `delta` (1 or -1) from `pos`, and whether the step wrapped round the end. */
export function step(
  sets: SectionMatches,
  pos: FindPos | undefined,
  delta: 1 | -1,
): { to: FindPos; wrapped: boolean } | undefined {
  const all = sets.flatMap((set, section) => (set ?? []).map((m) => posOf(section, m)));
  if (!all.length) return undefined;
  if (!pos) return { to: delta > 0 ? all[0] : all[all.length - 1], wrapped: false };
  const here = all.findIndex((p) => compare(p, pos) === 0);
  if (here >= 0) {
    const to = here + delta;
    return to >= 0 && to < all.length
      ? { to: all[to], wrapped: false }
      : { to: all[delta > 0 ? 0 : all.length - 1], wrapped: true };
  }
  // The place is no match any more: the next one from it, in the direction asked.
  const after = all.findIndex((p) => compare(p, pos) > 0);
  if (delta > 0) return after >= 0 ? { to: all[after], wrapped: false } : { to: all[0], wrapped: true };
  const before = (after < 0 ? all.length : after) - 1;
  return before >= 0 ? { to: all[before], wrapped: false } : { to: all[all.length - 1], wrapped: true };
}
