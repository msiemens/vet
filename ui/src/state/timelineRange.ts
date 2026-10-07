// The range of versions the review view compares, and how the timeline's choices change it.

import type { VersionInfo } from "../generated/VersionInfo";
import type { View } from "../generated/View";
import { CHANGESET, interdiffView } from "./view.ts";

/**
 * Version `from` against the later version `to`. A live end follows the working tree: it is
 * always the latest version, so a new settled version moves the range forward.
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

/**
 * The range a click on version `clicked` chooses: what changed since the version before it. With
 * `anchor`, the version clicked before, the range runs between the two. None for the first version,
 * which has none before it.
 */
export function rowRange(versions: readonly VersionInfo[], clicked: number, anchor?: number): Range | undefined {
  const first = versions.at(0)?.n;
  const latest = versions.at(-1)?.n;
  if (first === undefined || latest === undefined) return undefined;
  const [from, to] =
    anchor === undefined || anchor === clicked
      ? [clicked - 1, clicked]
      : [Math.min(anchor, clicked), Math.max(anchor, clicked)];
  return from < first ? undefined : { from, to, live: to === latest };
}

/** The range with its start moved to version `from`. */
export const withFrom = (range: Range, from: number): Range => ({ ...range, from });

/** The range with its end moved to version `to`: the latest version makes it live, an older one pins it. */
export function withTo(range: Range, to: number, latest: number): Range {
  return { ...range, to, live: to === latest };
}
