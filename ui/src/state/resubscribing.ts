// A listener on whatever is on view, which moves to the next one when it is replaced. Plain TypeScript, so
// it needs no reactive collections.

/** Where the thing on view and its replacement are heard of. */
export interface Holder<S> {
  /** What is on view now; none if nothing is. */
  now(): S | undefined;
  /** Calls `listener` with what is on view whenever it is replaced, and with none when it ends. */
  onReplaced(listener: (next: S | undefined) => void): void;
}

/**
 * Makes a `subscribe` for listeners on the thing `holder` holds: `attach` puts a listener on one thing and
 * returns the function that takes it off. The listeners stay while the thing is replaced and are put on
 * the next one.
 */
export function resubscribing<S, L>(holder: Holder<S>, attach: (on: S, listener: L) => () => void) {
  const listeners = new Set<L>();
  const attached = new Map<L, () => void>();
  holder.onReplaced((next) => {
    for (const detach of attached.values()) detach();
    attached.clear();
    if (next) for (const listener of listeners) attached.set(listener, attach(next, listener));
  });
  return (listener: L): (() => void) => {
    listeners.add(listener);
    const now = holder.now();
    if (now) attached.set(listener, attach(now, listener));
    return () => {
      listeners.delete(listener);
      attached.get(listener)?.();
      attached.delete(listener);
    };
  };
}
