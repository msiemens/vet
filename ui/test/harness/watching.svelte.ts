// What a component does to show the list: watch it from an effect, as the changeset switcher does while
// it is open. Runes need a `.svelte.ts` module.

import { untrack } from "svelte";
import { queue } from "../../src/state/queue.svelte.ts";

/** Lists and watches the queue from an effect of its own; returns the function that ends it. */
export function watchFromEffect(): () => void {
  return $effect.root(() => {
    $effect(() => {
      untrack(() => void queue.loadAll());
      return queue.watch();
    });
  });
}
