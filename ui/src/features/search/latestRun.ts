// A search that is asked again gives up on the one before it: its request is cancelled, so the backend
// does not go on reading files for an answer nobody wants, and its answer is dropped if it comes anyway.

import { isAbort } from "../../lib/abort.ts";

/** How a run ended; none if a later run or `cancel` replaced it. */
export type Outcome<T> = { type: "done"; value: T } | { type: "failed"; error: unknown };

export class LatestRun<T> {
  private current?: AbortController;

  /** Runs `ask`, after cancelling the run before it. */
  async run(ask: (signal: AbortSignal) => Promise<T>): Promise<Outcome<T> | undefined> {
    this.current?.abort();
    const mine = new AbortController();
    this.current = mine;
    let outcome: Outcome<T>;
    try {
      outcome = { type: "done", value: await ask(mine.signal) };
    } catch (error) {
      if (isAbort(error)) return undefined;
      outcome = { type: "failed", error };
    }
    return mine.signal.aborted ? undefined : outcome;
  }

  /** Gives up on the run in flight. */
  cancel() {
    this.current?.abort();
  }
}
