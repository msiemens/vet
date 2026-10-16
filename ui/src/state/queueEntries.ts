// The queue's rows, free of components: the local changesets and the pull requests as one list, and
// each row with the lines and progress it shows, the changeset on view's live. The backend lists the
// local rows and the platform the pull requests; the queue store joins them and merges the selected
// row's overlay once, so the queue, the changeset switcher and Search Everywhere read one list.

import type { ChangesetRow } from "../generated/ChangesetRow";
import type { Completion } from "../generated/Completion";
import type { LineDelta } from "../generated/LineDelta";
import type { PullRequestRow } from "../generated/PullRequestRow";
import type { RowReview } from "../generated/RowReview";
import { keyId } from "./changesetKey.ts";

/** A row of the queue: a local changeset, or a pull request with what the platform lists of it. */
export type QueueRow = ChangesetRow & { pullRequest?: PullRequestRow };

/** The queue's rows: the local changesets, then the pull requests. */
export function queueRows(local: readonly ChangesetRow[], pullRequests: readonly PullRequestRow[]): QueueRow[] {
  return [...local, ...pullRequests.map(pullRequestRow)];
}

/** A pull request as a row, which has no threads of its own listed: its comments are the platform's count. */
export function pullRequestRow(pullRequest: PullRequestRow): QueueRow {
  return {
    key: pullRequest.key,
    title: pullRequest.title,
    checkedOut: false,
    openThreads: 0,
    review: pullRequest.review,
    pullRequest,
  };
}

/** What the Changes column says of a row. */
export type Changes =
  /** The backend has not counted the lines yet. */
  | { kind: "pending" }
  /** There are none: nothing changed, or the row has no lines to count. */
  | { kind: "none" }
  | { kind: "lines"; lines: LineDelta };

/**
 * The lines of a row, from the most current source: `own`, the live manifest, when the row is the
 * changeset on view; else `counted`, what the backend counted between the commits of the row's base
 * and tip (`null`: there are none to count), the same rule the review view uses; else what the
 * platform lists of a pull request, until the backend's count replaces it (D93). The working tree
 * has its manifest only.
 */
export function changesOf(row: QueueRow, own: LineDelta | undefined, counted: LineDelta | null | undefined): Changes {
  const known = own ?? counted ?? row.pullRequest?.lines;
  if (known) return known.added + known.removed > 0 ? { kind: "lines", lines: known } : { kind: "none" };
  return row.key.type === "workingTree" || counted === null ? { kind: "none" } : { kind: "pending" };
}

/** What the progress column shows of a row. */
export type Reviewed =
  /** Not started. */
  | { kind: "none" }
  /** A review is open and its progress is not known here. */
  | { kind: "started" }
  | { kind: "progress"; seen: number; oldBase: number }
  /** Finished; `percent` is known when it was complete. */
  | { kind: "finished"; percent?: number };

/**
 * `completion` is the live progress of the changeset on view; any other row has the progress stored with
 * its review, if that was ever measured.
 */
export function reviewedOf(
  review: RowReview,
  completion?: Pick<Completion, "seenPercent" | "oldBasePercent">,
): Reviewed {
  switch (review.state) {
    case "none":
      return { kind: "none" };
    case "open": {
      const progress = completion ?? review.progress;
      return progress
        ? { kind: "progress", seen: progress.seenPercent, oldBase: progress.oldBasePercent }
        : { kind: "started" };
    }
    case "finished":
      return review.complete ? { kind: "finished", percent: 100 } : { kind: "finished" };
  }
}

/**
 * The changeset on view's live numbers, which only its row shows: the delta of its whole and its
 * completion. What the store holds as the frame of that view (state/reviewModel.ts) fits.
 */
export interface Overlay {
  readonly layout: { readonly delta: LineDelta };
  readonly facts: { readonly completion: Completion };
}

/** A queue row with the lines and progress it shows. */
export interface QueueEntry extends QueueRow {
  /** This row is the changeset on view. */
  onView: boolean;
  changes: Changes;
  reviewed: Reviewed;
}

/**
 * The rows with the lines and progress they show. The changeset on view's are live: its delta from
 * `own`, its completion and the lines the backend counted last; any other row has its stored review
 * and counted lines.
 */
export function queueEntries(
  rows: readonly QueueRow[],
  selectedId: string | undefined,
  own: Overlay | undefined,
  lines: ReadonlyMap<string, LineDelta | null>,
): QueueEntry[] {
  return rows.map((row) => {
    const id = keyId(row.key);
    const onView = id === selectedId;
    const live = onView ? own : undefined;
    return {
      ...row,
      onView,
      changes: changesOf(row, live?.layout.delta, lines.get(id)),
      reviewed: reviewedOf(row.review, live?.facts.completion),
    };
  });
}
