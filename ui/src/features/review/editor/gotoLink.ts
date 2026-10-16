// When a symbol is shown as a link to its definition (SPEC §8.4): while the modifier is held and the
// pointer rests on a symbol that has a definition. Whether it has one is asked of the server once the
// pointer has rested, and the question is dropped when the pointer moves on or the modifier is let go.

import type { HoverKeyed } from "./hoverPopup.ts";

/** How long the pointer rests on a symbol, modifier held, before the definition is asked for. */
export const LINK_MS = 80;

export interface LinkHost<T extends HoverKeyed> {
  /** Whether the symbol has a definition; rejects with an abort when `signal` aborts. */
  hasDefinition(target: T, signal: AbortSignal): Promise<boolean>;
  /** Marks the symbol as a link, or nothing when there is none. */
  show(target: T | undefined): void;
}

export class GotoLink<T extends HoverKeyed> {
  /** The symbol the question was settled for, and whether it is shown as a link. */
  private settled?: { key: string; shown: boolean };
  private pending?: { key: string; timer: ReturnType<typeof setTimeout>; abort: AbortController };

  private readonly host: LinkHost<T>;

  constructor(host: LinkHost<T>) {
    this.host = host;
  }

  /** The pointer or the modifier changed: `target` is the symbol under the pointer, if any. */
  update(target: T | undefined, modifier: boolean) {
    const wanted = modifier ? target : undefined;
    if (wanted && (wanted.key === this.settled?.key || wanted.key === this.pending?.key)) return;
    this.cancel();
    if (this.settled?.shown) this.host.show(undefined);
    this.settled = undefined;
    if (!wanted) return;
    const abort = new AbortController();
    this.pending = { key: wanted.key, abort, timer: setTimeout(() => void this.ask(wanted, abort), LINK_MS) };
  }

  /** Drops the link and the question, whatever the pointer does next. */
  clear() {
    this.update(undefined, false);
  }

  private async ask(target: T, abort: AbortController) {
    let found = false;
    try {
      found = await this.host.hasDefinition(target, abort.signal);
    } catch {
      // No answer is no link: an abort, or a server that cannot say.
    }
    if (abort.signal.aborted) return;
    this.pending = undefined;
    this.settled = { key: target.key, shown: found };
    if (found) this.host.show(target);
  }

  private cancel() {
    if (!this.pending) return;
    clearTimeout(this.pending.timer);
    this.pending.abort.abort();
    this.pending = undefined;
  }
}
