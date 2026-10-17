// The one place the UI sends a search, used by the Find in Files dialog and by each Find set.

import { Debounced } from "../lib/debounced.ts";
import type { Backend, SearchQuery, SearchResults } from "./backend.ts";
import { LatestRun, type Outcome } from "./latestRun.ts";

export class SearchRun {
  private readonly latest = new LatestRun<SearchResults>();
  private readonly pause = new Debounced();
  private waiting?: (outcome: undefined) => void;

  /** Searches for `query` after `delay` ms; the run before, waiting or in flight, is given up at once. */
  ask(backend: Pick<Backend, "search">, query: SearchQuery, delay = 0): Promise<Outcome<SearchResults> | undefined> {
    this.cancel();
    const start = () => this.latest.run((signal) => backend.search(query, signal));
    if (delay <= 0) return start();
    return new Promise((resolve) => {
      this.waiting = resolve;
      this.pause.schedule(delay, () => {
        this.waiting = undefined;
        void start().then(resolve);
      });
    });
  }

  /** Gives up the run waiting or in flight; its `ask` resolves `undefined`. */
  cancel() {
    this.pause.cancel();
    this.waiting?.(undefined);
    this.waiting = undefined;
    this.latest.cancel();
  }
}
