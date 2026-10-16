// The changeset's commits tagged with the revision they were read at, held in one place: the
// review's per-commit list, a tab that stays at a commit and the Find dialog all read from here, so
// commits are read once per revision.

import type { CommitUnit } from "../generated/CommitUnit";

/** Commits and the revision of the changeset they were read at. */
export interface CommitUnits {
  readonly revision: number;
  readonly units: CommitUnit[];
}

export class CommitList {
  private held?: CommitUnits;
  /** The read the callers of `at` may join, when there is one. */
  private reading?: Promise<CommitUnits>;
  /** Counts the reads started; only the newest-started one may replace `held`. */
  private started = 0;
  private read: () => Promise<{ revision: number; commits: CommitUnit[] }>;

  constructor(read: () => Promise<{ revision: number; commits: CommitUnit[] }>) {
    this.read = read;
  }

  /**
   * The commits of `revision`: the held list when it is of that revision, one read (joined by
   * callers asking meanwhile) when the held list is older, and nothing when the changeset has moved
   * past `revision`, which the caller treats as outdated.
   */
  async at(revision: number): Promise<CommitUnit[] | undefined> {
    const held = this.held;
    if (held?.revision === revision) return held.units;
    if (held && held.revision > revision) return undefined;
    const got = await this.readOnce(false);
    return got.revision === revision ? got.units : undefined;
  }

  /**
   * Reads the commits again, so progress after a mark is fresh. It starts its own read even when one
   * is underway: that one began before the mark and its answer lacks it.
   */
  async reload(): Promise<CommitUnits> {
    return await this.readOnce(true);
  }

  private readOnce(own: boolean): Promise<CommitUnits> {
    if (own || !this.reading) {
      const generation = ++this.started;
      const read = this.read()
        .then(({ revision, commits }) => {
          const got = { revision, units: commits };
          // An answer of an older read that arrives late is dropped, not held.
          if (generation === this.started) this.held = got;
          return got;
        })
        .finally(() => {
          if (this.reading === read) this.reading = undefined;
        });
      this.reading = read;
    }
    return this.reading;
  }
}
