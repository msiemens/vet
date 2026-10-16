// Following a store for the review screen's sake: what only that screen shows follows the project
// while the screen is up, and lets go when it goes. Kept out of `followProject.ts` so the changeset
// store can use it without a cycle (it is imported by `reviewShown`).

import type { Backend } from "./backend";
import { reviewShown } from "./reviewShown.svelte";
import { session } from "./session.svelte";
import { followShown } from "./whileShown";

/**
 * Runs `start` for the open repository while the review screen is shown, and the function it returns
 * when the screen goes away or the repository closes: what only the review screen shows follows the
 * screen, not the project, so it reads nothing for a screen nobody sees.
 */
export function whileReview(start: (backend: Backend) => () => void): void {
  followShown(start, {
    backend: () => (session.repo === undefined ? undefined : session.backend),
    shown: () => reviewShown.on,
    onOpen: (listener) => session.onOpen(listener),
    onClose: (listener) => session.onClose(listener),
    onShown: (listener) => reviewShown.onShown(listener),
    onHidden: (listener) => reviewShown.onHidden(listener),
  });
}
