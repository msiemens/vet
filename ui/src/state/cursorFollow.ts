// Following an editor's cursor with a question to the backend: it is asked once the cursor rests,
// so moving it does not flood the language server, and an answer to a place the cursor has left
// is dropped, its request cancelled.

import { isAbort } from "../lib/abort.ts";

export interface FollowOptions<Q, A> {
  /** How long the cursor rests before it is asked about. */
  ms: number;
  /** Equal keys are the same place. */
  key: (place: Q) => string;
  ask: (place: Q, signal: AbortSignal) => Promise<A>;
  /** The answer for the place the cursor is at. */
  apply: (answer: A, place: Q) => void;
  /** A request failed, other than by being cancelled. */
  fail: (error: unknown) => void;
}

export class CursorFollow<Q, A> {
  private timer?: ReturnType<typeof setTimeout>;
  private asking?: AbortController;
  /** The place asked about last, while its answer is still the one wanted. */
  private requested?: string;

  private readonly options: FollowOptions<Q, A>;

  constructor(options: FollowOptions<Q, A>) {
    this.options = options;
  }

  /** The cursor is at `place` now. */
  move(place: Q) {
    clearTimeout(this.timer);
    if (this.options.key(place) === this.requested) return;
    this.timer = setTimeout(() => this.ask(place), this.options.ms);
  }

  /** Forgets where the cursor was, so the next place is asked about again, and cancels what is underway. */
  reset() {
    clearTimeout(this.timer);
    this.asking?.abort();
    this.asking = this.requested = undefined;
  }

  private ask(place: Q) {
    this.asking?.abort();
    const asking = (this.asking = new AbortController());
    this.requested = this.options.key(place);
    // An `ask` that throws counts as a request that failed.
    new Promise<A>((resolve) => resolve(this.options.ask(place, asking.signal))).then(
      (answer) => {
        if (!asking.signal.aborted) this.options.apply(answer, place);
      },
      (error: unknown) => {
        if (asking.signal.aborted) return;
        // Asked again on the next move.
        this.requested = undefined;
        if (!isAbort(error)) this.options.fail(error);
      },
    );
  }
}
