/**
 * A call that waits: `schedule` runs it after `ms`, and scheduling again before then replaces it, so
 * only the last call of a burst runs. `cancel` drops the one waiting.
 */
export class Debounced {
  private handle: ReturnType<typeof setTimeout> | undefined;

  schedule(ms: number, run: () => void): void {
    this.cancel();
    this.handle = setTimeout(() => {
      this.handle = undefined;
      run();
    }, ms);
  }

  cancel(): void {
    clearTimeout(this.handle);
    this.handle = undefined;
  }

  /** A call is waiting. */
  get pending(): boolean {
    return this.handle !== undefined;
  }
}
