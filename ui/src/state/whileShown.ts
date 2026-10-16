// Following a project for one screen's sake: the data a store holds is wanted while the screen that
// shows it is up, and given up when it goes, so nothing of it is read for a screen nobody sees.

/** Where a store that follows the review screen hears of the screen. */
export interface ShownSources {
  /** Whether the screen is shown. */
  shown(): boolean;
  /** Calls `listener` whenever the screen comes up; returns the function that stops that. */
  onShown(listener: () => void): () => void;
  /** Calls `listener` whenever the screen goes away; returns the function that stops that. */
  onHidden(listener: () => void): () => void;
}

/**
 * Runs `start` while the screen is shown, and the function it returns when the screen goes away.
 * Registering while the screen already shows starts it at once; the start is not doubled for a
 * showing that arrives twice. Returns the function that ends the following: it stops what runs and
 * listens to the screen no more.
 */
export function followShown(start: () => () => void, sources: ShownSources): () => void {
  let stop: (() => void) | undefined;
  let ended = false;
  const begin = () => {
    if (ended || stop !== undefined || !sources.shown()) return;
    stop = start();
  };
  const end = () => {
    stop?.();
    stop = undefined;
  };
  const unsubscribe = [sources.onShown(begin), sources.onHidden(end)];
  begin();
  return () => {
    ended = true;
    for (const off of unsubscribe) off();
    end();
  };
}
