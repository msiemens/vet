// What the review screen keeps while the session it is mounted for changes: the tabs and positions each
// changeset of the repository had, and the diff mode chosen last. A screen is made for one session, so
// this is what a switch back to a changeset finds again.

import type { Mode } from "../../lib/viewMode.ts";
import { type Storage, ViewMemory } from "./changesetViews.ts";

export interface ReviewMemory {
  /** What each changeset of the repository had open when it was last on view. */
  views: ViewMemory;
  /** The diff mode chosen last, which a diff tab opens in. */
  diffMode: Mode;
}

/** The memory of the repository on view: the same object while it is the same repository, a fresh one for another. */
export function reviewMemory(storage: Storage): (repo: string | undefined) => ReviewMemory {
  let held: { repo: string | undefined; memory: ReviewMemory } | undefined;
  return (repo) => {
    if (!held || held.repo !== repo) held = { repo, memory: { views: new ViewMemory(storage), diffMode: "unified" } };
    return held.memory;
  };
}
