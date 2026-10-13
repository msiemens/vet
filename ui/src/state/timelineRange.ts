// The range of versions the review view compares, and how the timeline's choices change it.

import type { VersionInfo } from "../generated/VersionInfo";
import type { View } from "../generated/View";
import { CHANGESET, interdiffView } from "./view.ts";

/**
 * What the pushes after version `from` up to version `to` changed, compared with `from`, which is
 * the base the review started from for 0. A live end follows the working tree: it is always the
 * latest version, so a new settled version moves the range forward.
 */
export interface Range {
  from: number;
  to: number;
  live: boolean;
}

/** The range as a string, to tell what was read in which; empty for the changeset. */
export const rangeKey = (range: Pick<Range, "from" | "to"> | undefined): string =>
  range ? `${range.from}-${range.to}` : "";

/** What the review view reads for `range`: the interdiff, or without one the changeset. */
export const rangeView = (range: Range | undefined): View => (range ? interdiffView(range.from, range.to) : CHANGESET);

/** The range with its end on the latest version, which is `latest` now. */
export const followed = (range: Range, latest: number): Range => ({ ...range, to: latest });

/**
 * The range when the timeline opens: the version something was last marked in up to the latest,
 * live. The first version stands in while nothing was marked. None when there is nothing after it.
 */
export function defaultRange(versions: readonly VersionInfo[]): Range | undefined {
  const latest = versions.at(-1)?.n;
  if (latest === undefined) return undefined;
  const from = versions.find((v) => v.reviewedHere)?.n ?? versions[0].n;
  return from < latest ? { from, to: latest, live: true } : undefined;
}

/** The pushes a range shows: those after version `from`, which is the base for 0, up to `to`. */
export const pushesOf = (range: Range): { first: number; last: number } => ({
  first: range.from + 1,
  last: range.to,
});

/**
 * What a click on the row of version `clicked` shows, as a range; none for the whole changeset.
 * A click shows that push, what it changed from the version before it (the base for the first).
 * Clicking the only push shown goes back to the whole changeset. With `anchor`, the version clicked
 * before (a shift-click), it shows the block of pushes between the two. All the pushes are the
 * whole changeset, which is no range of its own.
 */
export function chooseRow(
  versions: readonly VersionInfo[],
  current: Range | undefined,
  clicked: number,
  anchor?: number,
): Range | undefined {
  const latest = versions.at(-1)?.n;
  if (latest === undefined) return undefined;
  const only = current !== undefined && current.from + 1 === clicked && current.to === clicked;
  if (anchor === undefined && only) return undefined;
  const first = Math.min(anchor ?? clicked, clicked);
  const last = Math.max(anchor ?? clicked, clicked);
  return first === 1 && last === latest ? undefined : { from: first - 1, to: last, live: last === latest };
}
