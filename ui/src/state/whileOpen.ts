import type { Backend } from "./backend";
import { session } from "./session.svelte";

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
