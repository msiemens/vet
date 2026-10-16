// What opening a screen asked for, held until the screen's controller is ready: a queue action that
// shows the review screen first must not be dropped while the controller is still being built.

/** A queue of operations waiting for a value to be ready, such as the review screen's controller. */
export class IntentQueue<T> {
  private pending: ((value: T) => void)[] = [];

  /** Runs `act` with `ready` now if it is there, else holds it until `ready` arrives. */
  when(act: (value: T) => void, ready: T | undefined): void {
    if (ready !== undefined) act(ready);
    else this.pending.push(act);
  }

  /** The value is ready: what was held runs in the order it came. */
  ready(value: T): void {
    const pending = this.pending;
    this.pending = [];
    for (const act of pending) act(value);
  }

  /** The screen is gone: what it still held goes with it. */
  clear(): void {
    this.pending = [];
  }
}
