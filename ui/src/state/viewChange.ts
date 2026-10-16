// What changed between two views of the changeset on screen, for the views built from them to know
// whether to follow in place or to start over.

import type { ViewLayout } from "./reviewModel";

/** What the views on screen show: the changeset's revision and which part of it. */
export interface ViewPoint {
  revision: number;
  /** The versions compared, if the view is a range. */
  range?: { from: number; to: number };
  /** The commit shown, if the view is one. */
  commit?: number;
  /** The files the Changes tab is limited to, if it is. Told apart by identity. */
  filtered?: ReadonlySet<string>;
}

/**
 * Nothing the views show changed; the same view of a newer revision (the views are rebuilt and keep
 * their place); another range, commit or filter (the views are rebuilt and the Changes tab starts at
 * the top).
 */
export type ViewChange = "none" | "revision" | "start";

export function viewPoint(
  layout: Pick<ViewLayout, "shown" | "interdiff">,
  commit: number | undefined,
  filtered: ReadonlySet<string> | undefined,
): ViewPoint {
  const { interdiff } = layout;
  return {
    revision: layout.shown.revision,
    range: interdiff && { from: interdiff.from, to: interdiff.to },
    commit,
    filtered,
  };
}

/**
 * How `next` differs from `prev`. No range and a range differ, and so do ranges that start at another
 * version; a range whose end moved with a new revision is the same range.
 */
export function viewChange(prev: ViewPoint | undefined, next: ViewPoint): ViewChange {
  if (!prev) return next.range || next.commit !== undefined || next.filtered ? "start" : "revision";
  if (anotherRange(prev, next) || prev.commit !== next.commit || prev.filtered !== next.filtered) return "start";
  return prev.revision === next.revision ? "none" : "revision";
}

function anotherRange(prev: ViewPoint, next: ViewPoint): boolean {
  const was = prev.range;
  const now = next.range;
  if (!was || !now) return !was !== !now;
  return now.from !== was.from || (now.to !== was.to && next.revision === prev.revision);
}
