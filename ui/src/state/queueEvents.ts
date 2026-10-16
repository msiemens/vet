// What the queue does about the backend's events: list what a fetch or a mark changed, show the
// lines of the rows as they come, and show what the details that were read add to a pull request
// row in place rather than list the pull requests again.

import type { ChangesetKey } from "../generated/ChangesetKey";
import type { LineDelta } from "../generated/LineDelta";
import type { PullRequestRow } from "../generated/PullRequestRow";
import type { PullRequestUpdate } from "../generated/PullRequestUpdate";
import type { SessionEvent } from "./backend.ts";
import { keyId } from "./changesetKey.ts";

/** What an event asks of the store, each as the store does it. */
export interface QueueActions {
  /** List the local changesets; `recount` false keeps the lines of the rows already counted. */
  load(recount?: boolean): void;
  /** List the pull requests again. */
  loadPullRequests(): void;
  /** Show those rows of the changeset's lines as they were counted. */
  takeLines(changesets: readonly ChangesetKey[], lines: readonly (LineDelta | null | undefined)[]): void;
  /** Show what the details that were read add to those pull request rows. */
  takeDetails(changed: readonly PullRequestUpdate[]): void;
  /** Follow the changeset on view after a new version or a mark. */
  follow(): void;
}

/** Does what `event` asks of the queue; an event the list shows nothing of does nothing. */
export function queueEvent(event: SessionEvent, actions: QueueActions): void {
  switch (event.type) {
    case "changesetOpened":
    case "reviewStarted":
      // Opening a changeset or starting a review moves no row of the platform's list: the local rows
      // are enough, so no `loadPullRequests`.
      actions.load();
      return;
    case "changesetsChanged":
      actions.load();
      return;
    case "threadsChanged":
      actions.load(false);
      return;
    case "pullRequestListChanged":
      actions.loadPullRequests();
      return;
    case "pullRequestsChanged":
      actions.takeDetails(event.changed);
      return;
    case "changesetLinesChanged":
      actions.takeLines(event.changesets, event.lines);
      return;
    case "snapshotSettled":
    case "reviewChanged":
      actions.follow();
      return;
    default:
      return;
  }
}

/** The pull request rows with what the details that were read add to the ones they changed. */
export function withDetails(rows: readonly PullRequestRow[], changed: readonly PullRequestUpdate[]): PullRequestRow[] {
  if (changed.length === 0) return [...rows];
  const byKey = new Map(changed.map((update) => [keyId(update.key), update]));
  return rows.map((row) => {
    const update = byKey.get(keyId(row.key));
    if (update === undefined) return row;
    return {
      ...row,
      status: update.status,
      unresolvedThreads: update.unresolvedThreads,
      changesRequested: update.changesRequested,
    };
  });
}
