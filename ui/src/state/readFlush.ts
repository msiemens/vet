// Runs the reads a session's events asked for, once per burst and in a fixed order: the manifest, the
// whole changeset unless the manifest fed it, then threads and the rest started together without waiting
// for them, then the file-tab scopes, which read after the manifest they depend on.

import { Debounced } from "../lib/debounced.ts";
import type { Lifetime } from "./lifetime.ts";
import { NO_READS, type Reads, isEmpty, merge } from "./sessionReads.ts";
import type { EventRevision } from "./wholeFetch.ts";

/** What the flush starts. Each read goes through the store's own wrapper, where only the latest counts. */
export interface ReadActions {
  /** Reads the manifest; resolves with whether that fed the whole-changeset view. */
  manifest(): Promise<boolean>;
  whole(revision: EventRevision): Promise<unknown>;
  threads(): Promise<unknown>;
  pullRequest(): Promise<unknown>;
  versions(): Promise<unknown>;
  changesSince(): void;
  commits(): Promise<unknown>;
  scopes(): void;
}

export class ReadFlush {
  private wanted = NO_READS;
  private timer = new Debounced();
  private running = false;
  /** Reads started and not yet settled. */
  private underway = 0;
  private actions: ReadActions;
  private lifetime: Lifetime;

  constructor(actions: ReadActions, lifetime: Lifetime) {
    this.actions = actions;
    this.lifetime = lifetime;
    lifetime.add(() => {
      this.timer.cancel();
      this.wanted = NO_READS;
    });
  }

  /**
   * Asks for `reads` on the next task. What is asked while a pass runs is held for the pass after it,
   * not started beside it.
   */
  want(reads: Reads) {
    if (!this.lifetime.alive || isEmpty(reads)) return;
    this.wanted = merge(this.wanted, reads);
    if (!this.running) this.timer.schedule(0, () => void this.run());
  }

  /** Reads are waiting, or a pass or a read it started is underway. */
  get pending(): boolean {
    return this.timer.pending || this.running || this.underway > 0;
  }

  private track(read: Promise<unknown>) {
    this.underway++;
    void read.then(
      () => this.underway--,
      () => this.underway--,
    );
  }

  /** A call, so that the checks after an await are not taken for always false. */
  private dead(): boolean {
    return !this.lifetime.alive;
  }

  private async run() {
    this.running = true;
    try {
      while (!this.dead() && !isEmpty(this.wanted)) {
        const reads = this.wanted;
        this.wanted = NO_READS;
        const fed = reads.manifest && (await this.actions.manifest());
        if (this.dead()) return;
        if (reads.whole && !fed) await this.actions.whole(reads.whole.revision);
        if (this.dead()) return;
        // Nothing below is awaited: the next pass does not depend on it, and the stores keep only the latest read.
        if (reads.threads) this.track(this.actions.threads());
        if (reads.pullRequest) this.track(this.actions.pullRequest());
        if (reads.versions) this.track(this.actions.versions());
        if (reads.changesSince) this.actions.changesSince();
        if (reads.commits) this.track(this.actions.commits());
        if (reads.scopes) this.actions.scopes();
      }
    } finally {
      this.running = false;
    }
  }
}
