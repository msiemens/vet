// What differs between a version of the open review and the current one, as the backend compares
// them (D95): a change taken back and what only a base move explains count for nothing. The queue
// panel asks for the version the review last marked; Find asks the backend itself, for the version
// its result set ran at.

import { SvelteMap } from "svelte/reactivity";
import type { Backend } from "./backend";
import { followProject, whileOpen } from "./followProject";

/** How many files differ between two versions, and which versions they are. */
export interface ChangeCount {
  from: number;
  to: number;
  files: number;
}

class ReviewChangesStore {
  private backend?: Backend;
  /** The answer per version asked for; an event that moves the versions clears it. */
  private answers = new SvelteMap<number, ChangeCount>();
  private asking = new Set<number>();

  constructor() {
    whileOpen((backend) => this.start(backend));
  }

  private start(backend: Backend): () => void {
    this.backend = backend;
    const stop = followProject(backend, {
      // A new version, a started review or a mark moves where the review last was: ask again.
      reloadOn: ["snapshotSettled", "reviewStarted", "reviewChanged"],
      reload: () => this.reload(),
      cancel: () => undefined,
      clear: () => this.answers.clear(),
    });
    return () => {
      stop();
      this.backend = undefined;
      this.answers.clear();
      this.asking.clear();
    };
  }

  /** How many files differ between version `version` and the current one, once it is known. */
  of(version: number | undefined): ChangeCount | undefined {
    return version === undefined ? undefined : this.answers.get(version);
  }

  /** Asks the backend for `version`, unless the answer is known or already on its way. */
  ensure(version: number | undefined) {
    const backend = this.backend;
    if (version === undefined || !backend || this.answers.has(version) || this.asking.has(version)) return;
    this.asking.add(version);
    void backend
      .changedSince(version)
      .then(
        (changed) => {
          if (this.backend === backend) {
            this.answers.set(version, { from: changed.from, to: changed.to, files: changed.files.length });
          }
        },
        () => undefined,
      )
      .finally(() => this.asking.delete(version));
  }

  /** Asks again for every version asked about so far: an event moved where the changes are. */
  private reload() {
    const asked = [...this.answers.keys()];
    this.answers.clear();
    for (const version of asked) this.ensure(version);
  }
}

export const reviewChanges = new ReviewChangesStore();
