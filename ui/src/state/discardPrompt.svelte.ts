// The open question of whether comment text may be discarded, which `DiscardDialog` shows and answers.
// Askers that come while it is open wait for the same answer.

class DiscardPrompt {
  /** How many comments the open question is about; none while there is no question. */
  count = $state<number>();
  private waiting: ((discard: boolean) => void)[] = [];

  /** Asks, resolving with whether the text may be discarded. */
  ask = (count: number): Promise<boolean> => {
    this.count = count;
    return new Promise((resolve) => this.waiting.push(resolve));
  };

  /** The viewer chose: Discard, or Keep editing. */
  answer(discard: boolean) {
    this.count = undefined;
    for (const resolve of this.waiting.splice(0)) resolve(discard);
  }
}

export const discardPrompt = new DiscardPrompt();
