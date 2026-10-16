// What differs between a version of a session's open review and the current one, as the backend
// compares them (D95): a change taken back and what only a base move explains count for nothing. The
// queue panel asks for the version the review last marked; Find asks the backend itself, for the version
// its result set ran at. It lives as long as the session: the queue screen shows it too.

import { SvelteMap } from "svelte/reactivity";
import type { Backend } from "./backend";

/** How many files differ between two versions, and which versions they are. */
export interface ChangeCount {
  from: number;
  to: number;
  files: number;
}

class ReviewChanges {
  /** The answer per version asked for; an event that moves the versions clears it. */
  private answers = new SvelteMap<number, ChangeCount>();
  private asking = new Set<number>();
  private ended = false;
  private backend: () => Backend;

  constructor(backend: () => Backend) {
    this.backend = backend;
  }

  /** The session ended: answers still on their way count for nothing. */
  end() {
    this.ended = true;
    this.answers.clear();
    this.asking.clear();
  }

  /** How many files differ between version `version` and the current one, once it is known. */
  of(version: number | undefined): ChangeCount | undefined {
    return version === undefined ? undefined : this.answers.get(version);
  }

  /** Asks the backend for `version`, unless the answer is known or already on its way. */
  ensure(version: number | undefined) {
    if (version === undefined || this.ended || this.answers.has(version) || this.asking.has(version)) return;
    this.asking.add(version);
    void this.backend()
      .changedSince(version)
      .then(
        (changed) => {
          if (!this.ended)
            this.answers.set(version, { from: changed.from, to: changed.to, files: changed.files.length });
        },
        () => undefined,
      )
      .finally(() => this.asking.delete(version));
  }

  /** Asks again for every version asked about so far: the versions or where the review last was moved. */
  reload() {
    const asked = [...this.answers.keys()];
    this.answers.clear();
    for (const version of asked) this.ensure(version);
  }
}

export type SessionReviewChanges = ReviewChanges;

export const newReviewChanges = (backend: () => Backend): SessionReviewChanges => new ReviewChanges(backend);
