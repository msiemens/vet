// Which reads a changeset's session needs after an event of its project. The table is pure; the session
// collects what it says and runs the reads in one flush (see `readFlush`).

import type { SessionEvent } from "./backend.ts";
import type { EventRevision } from "./wholeFetch.ts";

/** The reads an event asks for. */
export interface Reads {
  /** The manifest of the view on screen. */
  manifest: boolean;
  /** The whole-changeset view, unless the manifest fed it; `revision` is the one the event named. */
  whole?: { revision: EventRevision };
  threads: boolean;
  pullRequest: boolean;
  versions: boolean;
  /** What differs since each version asked about. */
  changesSince: boolean;
  /** The per-commit list with its progress. */
  commits: boolean;
  /** The scopes of the open file tabs. */
  scopes: boolean;
}

/** What decides which reads an event needs. */
export interface ReadState {
  /** Only the review screen shows the versions and the commits. */
  reviewShown: boolean;
  /** The session is of a pull request, the only changeset that asks the platform about it. */
  hasPullRequest: boolean;
  /** The revision of the view on screen. */
  shownRevision: number | undefined;
  /** The view on screen is the changeset as a whole, not a range or a commit. */
  showsChangeset: boolean;
  /** A range is on view. */
  inRange: boolean;
  /** The changeset is reviewed by commit and has more than one. */
  byCommit: boolean;
}

export const NO_READS: Reads = {
  manifest: false,
  threads: false,
  pullRequest: false,
  versions: false,
  changesSince: false,
  commits: false,
  scopes: false,
};

/** Whether `reads` asks for nothing. */
export function isEmpty(reads: Reads): boolean {
  return (
    !reads.manifest &&
    !reads.whole &&
    !reads.threads &&
    !reads.pullRequest &&
    !reads.versions &&
    !reads.changesSince &&
    !reads.commits &&
    !reads.scopes
  );
}

/** Everything either asks for. */
export function merge(a: Reads, b: Reads): Reads {
  const whole = a.whole && b.whole ? { revision: laterOf(a.whole.revision, b.whole.revision) } : (a.whole ?? b.whole);
  return {
    manifest: a.manifest || b.manifest,
    whole,
    threads: a.threads || b.threads,
    pullRequest: a.pullRequest || b.pullRequest,
    versions: a.versions || b.versions,
    changesSince: a.changesSince || b.changesSince,
    commits: a.commits || b.commits,
    scopes: a.scopes || b.scopes,
  };
}

/** The revision that makes the whole-changeset view stale in more cases: none names no revision and moves it regardless. */
function laterOf(a: EventRevision, b: EventRevision): EventRevision {
  return a === undefined || b === undefined ? undefined : Math.max(a, b);
}

/** The reads `event` needs of a session in `state`. */
export function readsFor(event: SessionEvent, state: ReadState): Reads {
  switch (event.type) {
    case "snapshotSettled":
      return {
        ...NO_READS,
        manifest: true,
        whole: { revision: event.revision },
        threads: true,
        versions: state.reviewShown,
        changesSince: true,
        scopes: true,
      };
    case "targetChanged":
      return { ...NO_READS, manifest: true, whole: { revision: undefined }, scopes: true };
    // Leaving the by commit lens while a range is on view keeps the range: nothing to read.
    case "lensChanged":
      return { ...NO_READS, manifest: event.lens === "perCommit" || !state.inRange };
    case "threadsChanged":
      return { ...NO_READS, threads: true, pullRequest: state.hasPullRequest };
    case "platformStatus":
      return { ...NO_READS, pullRequest: state.hasPullRequest };
    case "reviewStarted":
      return { ...NO_READS, versions: state.reviewShown, changesSince: true };
    case "reviewChanged":
      // A mark made in a range or a commit is announced as that of the changeset's file, which moves the commits' progress.
      return {
        ...NO_READS,
        versions: state.reviewShown,
        changesSince: true,
        commits: event.revision === state.shownRevision && (state.showsChangeset ? state.byCommit : true),
      };
    default:
      return NO_READS;
  }
}
