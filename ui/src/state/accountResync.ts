/** What a change of accounts asks of the platform store; the store provides it. */
export interface ResyncSteps {
  /** Syncs the threads with the platform; reports a failure itself. */
  sync(): Promise<void>;
  /** Asks again what the platform said of the pull request. */
  reload(): Promise<void>;
}

/**
 * Syncs and reloads after the accounts changed. A change that arrives while that runs is not lost
 * (the credential it brings may be the one the run in flight did not have) and not run in parallel:
 * however many arrive, one more run follows.
 */
export class AccountResync {
  private steps: ResyncSteps;
  private running = false;
  private changes = 0;

  constructor(steps: ResyncSteps) {
    this.steps = steps;
  }

  async changed(): Promise<void> {
    this.changes++;
    if (this.running) return;
    this.running = true;
    try {
      let seen: number;
      do {
        seen = this.changes;
        await this.steps.sync();
        await this.steps.reload();
      } while (this.changes !== seen);
    } finally {
      this.running = false;
    }
  }
}
