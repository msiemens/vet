// How the matches of a file reach its editors: which ranges each editor marks in each view mode,
// and the summary the find bar shows.

import type { Mode } from "../../../lib/viewMode.ts";
import {
  type FindMatch,
  type FindOptions,
  type FindPos,
  type FindSide,
  type SectionMatches,
  indexOf,
  posOf,
  totalOf,
} from "./findMatches.ts";

/** One match of a line: columns of the line. */
export interface LineMark {
  line: number;
  from: number;
  to: number;
  current: boolean;
}

/** What one editor marks. */
export interface ViewMarks {
  /** In the editor's document. */
  ranges: readonly LineMark[];
  /** In the deleted lines of a unified diff, by line of the base text. */
  removed: readonly LineMark[];
  /** The marks are of the other side's lines (a context line seen from a side that was not asked for). */
  dim: boolean;
}

export const NO_MARKS: ViewMarks = { ranges: [], removed: [], dim: false };

/** What one editor of a view marks, for each of the file's editors (the base one only in a side by side view). */
export interface EditorMarks {
  target: ViewMarks;
  base: ViewMarks;
}

/**
 * The marks of a file's matches. A context line is on both sides: a side by side view marks both
 * copies, and the copy of the side that was not asked for is dimmed. The current match is the one at
 * `current`.
 */
export function editorMarks(
  section: number,
  matches: readonly FindMatch[],
  current: FindPos | undefined,
  mode: Mode | "file",
  side: FindSide,
): EditorMarks {
  const isCurrent = (m: FindMatch) =>
    current?.section === section && current.order === m.order && current.from === m.from;
  const mark = (m: FindMatch, line: number): LineMark => ({ line, from: m.from, to: m.to, current: isCurrent(m) });
  const inDocument = matches.filter((m) => m.kind !== "removed").map((m) => mark(m, m.line));
  const removed = matches.filter((m) => m.kind === "removed");
  if (mode === "unified" || mode === "file")
    return {
      target: { ranges: inDocument, removed: removed.map((m) => mark(m, m.baseLine)), dim: false },
      base: NO_MARKS,
    };
  const base = matches.filter((m) => m.kind !== "added").map((m) => mark(m, m.baseLine));
  return {
    target: { ranges: inDocument, removed: [], dim: side === "base" },
    base: { ranges: base, removed: [], dim: side === "target" },
  };
}

/** What the find bar shows of a view's find. */
export interface FindSummary {
  total: number;
  /** The 0-based position of the current match among those found so far; -1 for none. */
  index: number;
  /** Matches in lines the change did not touch. */
  unchanged: number;
  /** Sections whose text has not been searched yet. */
  pending: number;
  /** Files too large to search. */
  skipped: number;
  /** The pattern the engine rejects, with its reason. */
  error?: string;
  /** Where the current match is. */
  at?: FindPos;
}

/** A view that can search what it shows: the diff of a changeset or of a file, or a file's text. */
export interface Finder {
  /** Who hears of what the find found, now and after each change to it. */
  onFind(listener: (summary: FindSummary) => void): void;
  /** Starts a find, or ends it (none); the current match is the first from `from`, else from where the view is. */
  find(options: FindOptions | undefined, from?: FindPos): void;
  /** Goes to the match `delta` from the current one; answers whether the step wrapped, or none without matches. */
  findStep(delta: 1 | -1): Promise<{ wrapped: boolean } | undefined> | { wrapped: boolean } | undefined;
  /** Scrolls the current match into view, once the text it is in has been searched. */
  showFound(): Promise<void> | void;
  /** Puts the caret on the current match and the focus in its editor; answers whether there was one. */
  selectFound(): boolean;
}

export const NO_SUMMARY: FindSummary = { total: 0, index: -1, unchanged: 0, pending: 0, skipped: 0 };

export function summarize(sets: SectionMatches, at: FindPos | undefined, skipped: number): FindSummary {
  const found = sets.flatMap((set, section) => (set ?? []).map((m) => posOf(section, m)));
  const here = at && found.some((p) => p.section === at.section && p.order === at.order && p.from === at.from);
  return {
    total: totalOf(sets),
    index: here ? indexOf(sets, at) : -1,
    unchanged: sets.reduce((n, set) => n + (set?.filter((m) => m.kind === "context").length ?? 0), 0),
    pending: sets.filter((set) => set === undefined).length,
    skipped,
    at: here ? at : undefined,
  };
}
