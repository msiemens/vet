import type { Backend } from "./backend";
import { changeset } from "./changeset.svelte";
import { type Followed, keepCurrent } from "./keepCurrent";
import { session } from "./session.svelte";

/** Keeps a store of the open project's data right; see `keepCurrent`. */
export function followProject(backend: Backend, held: Followed): () => void {
  return keepCurrent(
    {
      onEvent: (listener) =>
        backend.onEvent((event) => {
          if (event.project === backend.project) listener(event);
        }),
      onResync: (listener) => session.onResync(() => listener()),
      onOpening: (listener) => changeset.onOpening(listener),
    },
    held,
  );
}

/**
 * Runs `start` for each repository that is opened, and the function it returns when that repository
 * is closed: a store of the project's data follows the project, not the screen that shows it.
 */
export function whileOpen(start: (backend: Backend) => () => void): void {
  let stop: (() => void) | undefined;
  session.onOpen((backend) => {
    stop?.();
    stop = start(backend);
  });
  session.onClose(() => {
    stop?.();
    stop = undefined;
  });
}
