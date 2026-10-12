// What Show Usages and Go to Implementation do with the places the backend finds: one place is
// gone to, none is said, several are listed in the popup. The popup opens only once the search has
// taken a moment, so a quick answer that is one place shows nothing in between.

import type { LocationRow } from "../generated/LocationRow.ts";
import type { LocationRows } from "../generated/LocationRows.ts";
import type { Side } from "../generated/Side.ts";
import type { CaretRect, PickerKind, PickerSink } from "./pickerTypes.ts";

/** How long a search runs before the popup shows "Searching…". */
const SHOW_AFTER_MS = 250;

/** What to ask and where the answer is shown. */
export interface Search {
  kind: PickerKind;
  /** The name at the caret. */
  symbol: string;
  anchor: CaretRect;
  side: Side;
  /** The revision of the view that asks. */
  revision: number;
  /** Asks the backend; `signal` aborts it. */
  ask(signal: AbortSignal): Promise<LocationRows>;
}

export interface SearchDeps {
  picker: PickerSink;
  /** Tells the viewer something in a toast. */
  notice(text: string): void;
  /** Goes to the place of `row`. */
  open(row: LocationRow, side: Side): void;
  /** Tells whether the error is the changeset moving on, which aborts what was asked of the old revision. */
  isAbort(error: unknown): boolean;
  /** Tells the viewer why a request failed that was sent for `revision`. */
  fail(error: unknown, revision: number): void;
}

/** Whose answer is the popup's: the newest search; an older one that comes in late is dropped. */
export class LocationSearch {
  private running?: { abort: AbortController };
  private deps: SearchDeps;
  private delayMs: number;

  constructor(deps: SearchDeps, delayMs = SHOW_AFTER_MS) {
    this.deps = deps;
    this.delayMs = delayMs;
  }

  /** Ends the search that runs, and closes what it showed. */
  cancel() {
    this.running?.abort.abort();
    this.running = undefined;
    this.deps.picker.close();
  }

  async start(search: Search): Promise<void> {
    this.running?.abort.abort();
    const mine = { abort: new AbortController() };
    this.running = mine;
    const { picker } = this.deps;
    picker.close();
    const current = () => this.running === mine;
    const shown = {
      kind: search.kind,
      symbol: search.symbol,
      anchor: search.anchor,
      side: search.side,
      onpick: (row: LocationRow) => {
        this.running = undefined;
        picker.close();
        this.deps.open(row, search.side);
      },
      onretry: () => void this.start(search),
      onclose: () => this.cancel(),
    };
    const timer = setTimeout(() => {
      if (current()) picker.show({ ...shown, body: { phase: "searching" } });
    }, this.delayMs);
    let answer: LocationRows;
    try {
      answer = await search.ask(mine.abort.signal);
    } catch (error) {
      clearTimeout(timer);
      if (!current()) return;
      this.running = undefined;
      picker.close();
      if (this.deps.isAbort(error)) this.deps.notice("Changes arrived — try again");
      else this.deps.fail(error, search.revision);
      return;
    }
    clearTimeout(timer);
    if (!current()) return;
    const { unavailable, rows, total } = answer;
    if (unavailable?.kind === "serverMissing" || unavailable?.kind === "serverFailed") {
      picker.show({ ...shown, body: { phase: "stopped", message: unavailable.message } });
      return;
    }
    this.running = undefined;
    if (unavailable) {
      picker.close();
      this.deps.notice(unavailable.message);
    } else if (total === 1 && rows.length === 1) {
      picker.close();
      this.deps.open(rows[0], search.side);
    } else {
      picker.show({ ...shown, body: { phase: "results", rows, total } });
    }
  }
}
