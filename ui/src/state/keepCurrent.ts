// What every store of the open project's data does to stay right: read at the start, read again
// after the events that change the data and after the connection came back, and let go of what it
// holds when another changeset is put on view.

import type { SessionEvent } from "./backend.ts";
import type { Opening } from "./changeset.svelte.ts";

/** Where a store that follows the open project hears of what changes it. */
export interface Upstream {
  /** Calls `listener` with each event of the open project; returns the function that stops that. */
  onEvent(listener: (event: SessionEvent) => void): () => void;
  /** Calls `listener` when the connection is up again, with the project open on it. */
  onResync(listener: () => void): () => void;
  /** Calls `listener` as a changeset is put on view, see `ChangesetStore.onOpening`. */
  onOpening(listener: (opening: Opening) => void): () => void;
}

/** The store that is kept right. */
export interface Followed {
  /** The events after which the data is read again. */
  reloadOn: readonly SessionEvent["type"][];
  /** Reads the data; the latest read counts. */
  reload(): void;
  /** The reads underway are of the changeset that is gone and count for nothing. */
  cancel(): void;
  /** What is held is of the changeset that is gone. */
  clear(): void;
  /** The store's own part of a changeset being put on view, before the common one. */
  opening?(opening: Opening): void;
  /** The store's own part of an event, before the common one. */
  event?(event: SessionEvent): void;
}

/**
 * Reads the data now and keeps it current. A changeset asked for drops what is held at once; the one
 * announced, or the changeset that stays on view after the ask came to nothing, is read anew.
 * Returns the function that stops following.
 */
export function keepCurrent(up: Upstream, held: Followed): () => void {
  held.reload();
  const stops = [
    up.onEvent((event) => {
      held.event?.(event);
      if (held.reloadOn.includes(event.type)) held.reload();
    }),
    up.onResync(() => held.reload()),
    up.onOpening((opening) => {
      held.opening?.(opening);
      held.cancel();
      if (opening.type !== "unannounced") held.clear();
      if (opening.type !== "requested") held.reload();
    }),
  ];
  return () => {
    for (const stop of stops) stop();
  };
}
