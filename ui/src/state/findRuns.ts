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

/** A set run on another revision than the one on show may list places that moved. A set of a commit's text never does. */
export function isStale(
  set: { done: boolean; commit: boolean; revision?: number },
  revision: number | undefined,
): boolean {
  return set.done && !set.commit && set.revision !== undefined && revision !== undefined && set.revision !== revision;
}
