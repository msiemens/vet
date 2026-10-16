// How long what belongs to one opened changeset lives. Whatever it starts registers its cleanup here, and
// every continuation of an asynchronous step asks whether the lifetime is still on before it touches state:
// an answer that arrives after the changeset was switched away from changes nothing.

/** Thrown by `check` after the lifetime ended; the step it interrupts is of a changeset that is gone. */
export class LifetimeEnded extends Error {
  constructor() {
    super("the changeset was switched away from");
  }
}

export class Lifetime {
  private ended = false;
  private cleanups: (() => void)[] = [];

  /** Whether the lifetime is still on. */
  get alive(): boolean {
    return !this.ended;
  }

  /** Throws `LifetimeEnded` once the lifetime is over, for a step that cannot stop quietly. */
  check(): void {
    if (this.ended) throw new LifetimeEnded();
  }

  /** Runs `cleanup` when the lifetime ends, in the reverse order of adding; at once if it has ended already. */
  add(cleanup: () => void): void {
    if (this.ended) cleanup();
    else this.cleanups.push(cleanup);
  }

  /** Ends the lifetime and runs the cleanups. Ending twice runs them once. */
  dispose(): void {
    if (this.ended) return;
    this.ended = true;
    const cleanups = this.cleanups.reverse();
    this.cleanups = [];
    for (const cleanup of cleanups) cleanup();
  }
}
