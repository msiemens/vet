/** What a `LatestLoad` does to the store that owns it. */
export interface LoadEffects<T> {
  /** The latest load started (`true`) or is over, with a result or an error (`false`). */
  busy(on: boolean): void;
  /** The latest load's result. */
  apply(value: T): void;
}

/**
 * Loads of one thing that may overlap (an event refetches while a request is underway; another
 * project is opened): only the latest counts. An older one that settles later changes nothing and
 * settles as the latest does. `cancel` makes the loads underway count for nothing, for a store
 * that was reset: they settle with `undefined`.
 */
export class LatestLoad<T> {
  private seq = 0;
  private latest?: Promise<T | undefined>;

  private effects: LoadEffects<T>;

  constructor(effects: LoadEffects<T>) {
    this.effects = effects;
  }

  load(fetch: () => Promise<T>): Promise<T | undefined> {
    const mine = ++this.seq;
    this.effects.busy(true);
    const run = (async () => {
      try {
        const value = await fetch();
        if (mine !== this.seq) return await this.latest;
        this.effects.apply(value);
        this.effects.busy(false);
        return value;
      } catch (error) {
        if (mine !== this.seq) return await this.latest;
        this.effects.busy(false);
        throw error;
      }
    })();
    this.latest = run;
    return run;
  }

  cancel() {
    this.seq++;
    this.latest = undefined;
    this.effects.busy(false);
  }
}
