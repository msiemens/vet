// Whether the review screen is the one on show. It stays mounted behind the queue, where choosing a
// row puts that changeset on view; the language servers of a changeset are for the review screen, so
// the backend is told when it is shown (SPEC §7.1) and what asks them in the background waits for it.

import { untrack } from "svelte";
import type { Backend } from "./backend";
import { changeset } from "./changeset.svelte";
import { ScreenReport } from "./screenReport";
import { session } from "./session.svelte";

class ReviewShown {
  on = $state(false);
  private listeners = new Set<() => void>();
  private hiddenListeners = new Set<() => void>();
  private report = new ScreenReport((shown) => void session.backend?.setReviewShown(shown).catch(() => undefined));
  private told?: { backend?: Backend; repo?: string };

  constructor() {
    $effect.root(() => {
      $effect(() => {
        const { backend, repo } = session;
        const on = this.on;
        const onView = !changeset.loading;
        untrack(() => this.tell(backend, repo, on, onView));
      });
    });
    // A backend that started anew knows nothing of what was shown.
    session.onResync(() => {
      this.report.forget();
      this.tell(session.backend, session.repo, this.on, !changeset.loading);
    });
  }

  /** The review screen came up or went away. */
  set(on: boolean) {
    if (on === this.on) return;
    this.on = on;
    if (on) for (const listener of this.listeners) listener();
    else for (const listener of this.hiddenListeners) listener();
  }

  /** Calls `listener` whenever the review screen comes up. Returns the function that stops that. */
  onShown(listener: () => void): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  /** Calls `listener` whenever the review screen goes away. Returns the function that stops that. */
  onHidden(listener: () => void): () => void {
    this.hiddenListeners.add(listener);
    return () => this.hiddenListeners.delete(listener);
  }

  private tell(backend: Backend | undefined, repo: string | undefined, on: boolean, onView: boolean) {
    if (backend !== this.told?.backend || repo !== this.told?.repo) {
      this.told = { backend, repo };
      this.report.forget();
    }
    if (backend && repo !== undefined) this.report.update(on, onView);
  }
}

export const reviewShown = new ReviewShown();
