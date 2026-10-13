// How result sets are kept and kept current, purely: which tab a new result goes to, what a rerun
// found new, and which files changed since a set ran.

import type { Place } from "./foundPlace.ts";

export type Slot = { kind: "rerun"; index: number } | { kind: "replace"; index: number } | { kind: "open" };

/**
 * Where a result goes: the tab of the same query if there is one (rerun, pinned or not, so a pinned
 * set is never copied), else the first tab that is not pinned, whose content it replaces, else a new
 * tab.
 */
export function slotFor(sets: readonly { key: string; pinned: boolean }[], key: string): Slot {
  const same = sets.findIndex((s) => s.key === key);
  if (same >= 0) return { kind: "rerun", index: same };
  const loose = sets.findIndex((s) => !s.pinned);
  return loose >= 0 ? { kind: "replace", index: loose } : { kind: "open" };
}

const sameThing = (p: Place) => `${p.file.path}|${p.text}|${p.col}`;

/** What a rerun found that the run before did not (by ids), and how many places it no longer finds. */
export function diffRuns(
  before: readonly Place[],
  after: readonly Place[],
): { fresh: ReadonlySet<string>; gone: number } {
  const had = new Set(before.map(sameThing));
  const has = new Set(after.map(sameThing));
  return {
    fresh: new Set(after.filter((p) => !had.has(sameThing(p))).map((p) => p.id)),
    gone: before.filter((p) => !has.has(sameThing(p))).length,
  };
}

/** The files that versions after `since` changed: what a set run on version `since` may be wrong about. */
export function changedSince(
  versions: readonly { n: number; files: readonly { path: string }[] }[],
  since: number | undefined,
): ReadonlySet<string> {
  if (since === undefined) return new Set();
  return new Set(versions.filter((v) => v.n > since).flatMap((v) => v.files.map((f) => f.path)));
}

/**
 * The place as it is opened once the lines of its file may have moved: at the line `moved` says. Only
 * a target line of a changed file is looked up; a line an edit replaced has no match to select.
 */
export function relocated(place: Place, moved: { line: number; kept: boolean } | undefined): Place {
  if (!moved) return place;
  return moved.kept ? { ...place, line: moved.line } : { ...place, line: moved.line, col: 0, width: 0 };
}

/** The lines of a file's target-side places to ask for, each once, in order. */
export function linesToLocate(places: readonly Place[], path: string): number[] {
  const lines = places.filter((p) => p.file.path === path && p.side === "target").map((p) => p.line);
  return [...new Set(lines)].sort((a, b) => a - b);
}

/** A set run on another revision than the one on show may list places that moved. A set of a commit's text never does. */
export function isStale(
  set: { done: boolean; commit: boolean; revision?: number },
  revision: number | undefined,
): boolean {
  return set.done && !set.commit && set.revision !== undefined && revision !== undefined && set.revision !== revision;
}
