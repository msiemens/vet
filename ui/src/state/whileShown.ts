// Following a project for one screen's sake: the data a store holds is wanted while the screen that
// shows it is up, and given up when it goes, so nothing of it is read for a screen nobody sees.

import type { Backend } from "./backend";

/** Where a store that follows the review screen hears of the project and the screen. */
export interface ShownSources {
  /** The backend of the open repository; none before one is open. */
  backend(): Backend | undefined;
  /** Whether the screen is shown. */
  shown(): boolean;
  /** Calls `listener` when a repository is opened. */
  onOpen(listener: () => void): void;
  /** Calls `listener` when the repository is closed. */
  onClose(listener: () => void): void;
  /** Calls `listener` whenever the screen comes up. */
  onShown(listener: () => void): void;
  /** Calls `listener` whenever the screen goes away. */
  onHidden(listener: () => void): void;
}

/**
 * Runs `start` for the open repository while the screen is shown, and the function it returns when
 * the screen goes away or the repository closes. Registering while the screen already shows starts it
 * at once; the start is not doubled for an opening and a showing that both arrive.
 */
export function followShown(start: (backend: Backend) => () => void, sources: ShownSources): void {
  let stop: (() => void) | undefined;
  const begin = () => {
    if (stop !== undefined || !sources.shown()) return;
    const backend = sources.backend();
    if (!backend) return;
    stop = start(backend);
  };
  const end = () => {
    stop?.();
    stop = undefined;
  };
  sources.onOpen(begin);
  sources.onClose(end);
  sources.onShown(begin);
  sources.onHidden(end);
  begin();
}
