// Lets at most `limit` tasks run at once; the others wait their turn in the order they came.

export class Gate {
  private readonly limit: number;
  private running = 0;
  private waiting: (() => void)[] = [];

  constructor(limit: number) {
    this.limit = limit;
  }

  async run<T>(task: () => Promise<T>): Promise<T> {
    if (this.running >= this.limit) await new Promise<void>((resume) => this.waiting.push(resume));
    this.running++;
    try {
      return await task();
    } finally {
      this.running--;
      this.waiting.shift()?.();
    }
  }
}
