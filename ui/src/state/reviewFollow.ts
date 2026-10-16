// Following a changeset's data for the review screen's sake: what only that screen shows is held while
// the screen is up, and lets go when it goes. Kept apart from the changeset session so it can be used
// without a cycle (it is imported by `reviewShown`).

import type { Lifetime } from "./lifetime";
import { reviewShown } from "./reviewShown.svelte";
import { followShown } from "./whileShown";

/**
 * Runs `start` while the review screen is shown, and what it returned when the screen goes away or
 * `lifetime` ends: what one opened changeset holds that only the review screen shows follows the
 * screen, so it reads nothing for a screen nobody sees.
 */
export function whileReviewShown(lifetime: Lifetime, start: () => () => void): void {
  lifetime.add(
    followShown(start, {
      shown: () => reviewShown.on && lifetime.alive,
      onShown: (listener) => reviewShown.onShown(listener),
      onHidden: (listener) => reviewShown.onHidden(listener),
    }),
  );
}
