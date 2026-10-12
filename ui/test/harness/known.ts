// The violations that are known bugs (see the stale-state audit), so the harness is green while they
// stand. A violation that matches no entry fails the run, and so does an entry nothing matched: fixing
// a bug means deleting its lines here. An entry names the session and step it happens in, so that it
// hides nothing else.

import type { View } from "../../src/generated/View.ts";

export interface Known {
  /** The bug's id in the audit. */
  bug: string;
  session: string;
  step: string;
  invariant: string;
  view: View["type"];
}

export const known: Known[] = [
  // Another window's marks never reach a window that shows a range or a commit.
  {
    bug: "H2",
    session: "otherWindow",
    step: "other marks while in the range",
    invariant: "fileReview",
    view: "interdiff",
  },
  {
    bug: "H2",
    session: "otherWindow",
    step: "other marks while in the range",
    invariant: "viewCompletion",
    view: "interdiff",
  },
  { bug: "H2", session: "otherWindow", step: "other marks while by commit", invariant: "fileReview", view: "commit" },
  {
    bug: "H2",
    session: "otherWindow",
    step: "other marks while by commit",
    invariant: "viewCompletion",
    view: "commit",
  },
];
