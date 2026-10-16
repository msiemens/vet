// Which of a session's thread lists serves which view. The changeset's threads are always held; a
// range's, which the backend resolves for the new side only, are held while a range is on view.

import type { Thread } from "../generated/Thread";
import { type Range, rangeKey } from "./timelineRange.ts";

/** The threads of one view, resolved at `revision`. Immutable, replaced as a whole. */
export interface ThreadCell {
  /** The manifest revision the anchors in `items` belong to. */
  revision: number;
  /** The range the threads were read in (see `rangeKey`); empty for the changeset. */
  key: string;
  items: Thread[];
}

export interface ThreadCells {
  /** The whole changeset's threads: the view of the changeset and of a commit, which has none of its own. */
  whole?: ThreadCell;
  /** The threads of the range on view. */
  range?: ThreadCell;
}

type Where = Pick<Range, "from" | "to">;

/** The cell that was read for the view of `range` (none for the changeset or a commit); none if that cell is not held. */
export function cellFor(cells: ThreadCells, range: Where | undefined): ThreadCell | undefined {
  if (!range) return cells.whole;
  return cells.range?.key === rangeKey(range) ? cells.range : undefined;
}

/** The cell a list of threads beside the view reads: the view's own, else the changeset's until the view's is read. */
export function cellListed(cells: ThreadCells, range: Where | undefined): ThreadCell | undefined {
  return cellFor(cells, range) ?? cells.whole;
}

/** The cell as of `revision`, its threads kept: for a revision that moved nothing the threads are anchored in. */
export function restamped(held: ThreadCell | undefined, revision: number): ThreadCell | undefined {
  return held && held.revision < revision ? { ...held, revision } : held;
}

/** Whether a read may replace `held`: an answer for an older revision does not replace a newer one. */
export function replaces(held: ThreadCell | undefined, read: ThreadCell): boolean {
  return !held || read.revision >= held.revision;
}
