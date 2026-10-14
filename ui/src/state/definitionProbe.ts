import { InFlight } from "./inFlight.ts";
import { Latest } from "./resource.ts";

/**
 * Whether a position has a definition, asked of the server once per position: an answer is kept
 * until `clear`, a question underway is shared, and a failure is no answer and is asked again.
 */
export class DefinitionProbe {
  private known = new Map<string, boolean>();
  private asking = new InFlight<boolean>();
  /** Moved by `clear`, so the answer to an earlier question is not kept. */
  private epoch = new Latest();

  private readonly ask: (key: string, signal: AbortSignal) => Promise<boolean>;

  /** `ask` answers whether the position `key` names has a definition. */
  constructor(ask: (key: string, signal: AbortSignal) => Promise<boolean>) {
    this.ask = ask;
  }

  async has(key: string, signal?: AbortSignal): Promise<boolean> {
    const known = this.known.get(key);
    if (known !== undefined) return known;
    const asked = this.epoch.mark();
    return this.asking.share(
      key,
      async (shared) => {
        const answer = await this.ask(key, shared);
        if (!this.epoch.movedSince(asked)) this.known.set(key, answer);
        return answer;
      },
      signal,
    );
  }

  clear() {
    this.epoch.cancel();
    this.known.clear();
    this.asking.clear();
  }
}
