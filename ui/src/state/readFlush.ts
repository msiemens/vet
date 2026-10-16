// Runs the reads a session's events asked for, once per burst and in a fixed order: the manifest, the
// whole changeset unless the manifest fed it, threads and the rest together, then the file-tab scopes,
// which read after the manifest they depend on.

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

  /** Reads are waiting or underway. */
  get pending(): boolean {
    return this.timer.pending || this.running;
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
        const together: Promise<unknown>[] = [];
        if (reads.threads) together.push(this.actions.threads());
        if (reads.pullRequest) together.push(this.actions.pullRequest());
        if (reads.versions) together.push(this.actions.versions());
        if (reads.changesSince) this.actions.changesSince();
        if (reads.commits) together.push(this.actions.commits());
        await Promise.allSettled(together);
        if (this.dead()) return;
        if (reads.scopes) this.actions.scopes();
      }
    } finally {
      this.running = false;
    }
  }
}
