// What a store needs of a thing it reads from the backend again and again (an event refetches it
// while a request is underway; another project is opened): only the latest read counts.

/** Tells whether the read it was begun for is still the latest, and so may change the store. */
export type IsLatest = () => boolean;

/**
 * The latest of reads that may overlap. `begin` starts one; `cancel` makes every read underway count
 * for nothing, for a store that was reset. For a store whose reads do more than set a value, which
 * checks `isLatest` where they have to; a store that sets a value uses a [`Resource`].
 *
 * Reads that run side by side and are all dropped at once (the answers of one revision) take a
 * `mark` instead of `begin`, and count while nothing `movedSince` it.
 */
export class Latest {
  private n = 0;

  begin(): IsLatest {
    const mine = ++this.n;
    return () => mine === this.n;
  }

  cancel() {
    this.n++;
  }

  /** Where the reads are now, for `movedSince`. */
  mark(): number {
    return this.n;
  }

  /** Whether a read began or was cancelled since `mark`. */
  movedSince(mark: number): boolean {
    return this.n !== mark;
  }
}

/** What a `Resource` does to the store that owns it. */
export interface ResourceEffects<T> {
  /** The latest read's result. */
  apply(value: T): void;
  /** The latest read failed. Without it, `load` rejects with the error. */
  failed?(error: unknown): void;
  /** The latest read started (`true`) or is over, with a result or an error (`false`). */
  busy?(on: boolean): void;
}

/**
 * A value read from the backend, of which only the latest read counts. An older read that settles
 * later changes nothing and settles as the latest does. `cancel` makes the reads underway count for
 * nothing: they settle with `undefined`.
 */
export class Resource<T> {
  private reads = new Latest();
  private latest?: Promise<T | undefined>;
  private inFlight = 0;
  private effects: ResourceEffects<T>;

  constructor(effects: ResourceEffects<T>) {
    this.effects = effects;
  }

  /** The read underway, for a caller that wants to join it rather than start another; none when idle. */
  get running(): Promise<T | undefined> | undefined {
    return this.inFlight > 0 ? this.latest : undefined;
  }

  load(read: () => Promise<T>): Promise<T | undefined> {
    const isLatest = this.reads.begin();
    this.effects.busy?.(true);
    this.inFlight++;
    const run = (async () => {
      try {
        const value = await read();
        if (!isLatest()) return await this.latest;
        this.effects.apply(value);
        this.effects.busy?.(false);
        return value;
      } catch (error) {
        if (!isLatest()) return await this.latest;
        this.effects.busy?.(false);
        if (!this.effects.failed) throw error;
        this.effects.failed(error);
        return undefined;
      } finally {
        this.inFlight--;
      }
    })();
    this.latest = run;
    return run;
  }

  cancel() {
    this.reads.cancel();
    this.latest = undefined;
    this.effects.busy?.(false);
  }
}
