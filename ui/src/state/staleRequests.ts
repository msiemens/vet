// The semantic requests underway, each for the revision of the view that asked. When the
// changeset moves to a newer revision their answers are unwanted (the backend would refuse
// them as stale), so they are aborted and the language server is spared the work.

export class StaleRequests {
  private underway = new Map<AbortController, number>();

  /**
   * A signal for a request about `revision`: it aborts when the changeset moves past that
   * revision, and when `signal` does. `release` ends the tracking once the request settled.
   */
  track(revision: number, signal?: AbortSignal): { signal: AbortSignal; release: () => void } {
    const controller = new AbortController();
    const abort = () => controller.abort();
    if (signal?.aborted) abort();
    signal?.addEventListener("abort", abort, { once: true });
    this.underway.set(controller, revision);
    return {
      signal: controller.signal,
      release: () => {
        this.underway.delete(controller);
        signal?.removeEventListener("abort", abort);
      },
    };
  }

  /** The changeset is at `revision` now: requests about an older one are aborted. */
  moveTo(revision: number) {
    for (const [controller, asked] of this.underway) {
      if (asked < revision) {
        this.underway.delete(controller);
        controller.abort();
      }
    }
  }
}
