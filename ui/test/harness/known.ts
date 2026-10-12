// The violations that are known bugs (see the stale-state audit), so the harness is green while they
// stand. A violation that matches no entry fails the run, and so does an entry nothing matched: fixing
// a bug means deleting its lines here.

import type { View } from "../../src/generated/View.ts";

export interface Known {
  /** The bug's id in the audit. */
  bug: string;
  invariant: string;
  view: View["type"];
}

export const known: Known[] = [
  // The Finish dialog's flagged count: a range never updates the changeset's totals.
  { bug: "P1", invariant: "changesetCompletion", view: "interdiff" },
  // A fetch writes the changeset's base over the base of a commit.
  { bug: "P2", invariant: "base", view: "commit" },
  // The queue takes the numbers of the commit on view as those of the changeset.
  { bug: "P3", invariant: "onViewRowFacts", view: "commit" },
  // The queue's on-view row ignores a settled version and a mark in a new version.
  { bug: "P5", invariant: "queueRow", view: "changeset" },
  { bug: "P5", invariant: "queueRow", view: "interdiff" },
  // The open editor views keep their review state across a reconnect.
  { bug: "P6", invariant: "viewMirror", view: "changeset" },
  // Another window's marks never reach a window that shows a range or a commit.
  { bug: "H2", invariant: "fileReview", view: "interdiff" },
  { bug: "H2", invariant: "viewCompletion", view: "interdiff" },
  { bug: "H2", invariant: "viewMirror", view: "interdiff" },
  { bug: "H2", invariant: "fileReview", view: "commit" },
  { bug: "H2", invariant: "viewCompletion", view: "commit" },
  { bug: "H2", invariant: "viewMirror", view: "commit" },
];
