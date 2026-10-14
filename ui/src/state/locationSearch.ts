// What Show Usages and Go to Implementation do with the places the backend finds: one place is
// gone to, none is said, several are listed in the popup. The popup opens only once the search has
// taken a moment, so a quick answer that is one place shows nothing in between.

import type { LocationRow } from "../generated/LocationRow.ts";
import type { LocationRows } from "../generated/LocationRows.ts";
import type { Side } from "../generated/Side.ts";
import { Debounced } from "../lib/debounced.ts";
import { STALE_NOTICE } from "../lib/notice.ts";
import type { CaretRect, PickerKind, PickerSink, PickerView } from "./pickerTypes.ts";

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
  /** Goes to the place of `row`, where the search was asked from. */
  open(row: LocationRow): void;
  /** Keeps the places the popup lists in the Find tool; absent where the popup offers no such thing. */
  keep?: (answer: LocationRows) => void;
}

export interface SearchDeps {
  picker: PickerSink;
  /** Tells the viewer something in a toast. */
  notice(text: string): void;
  /** Tells whether the error is the changeset moving on, which aborts what was asked of the old revision. */
  isAbort(error: unknown): boolean;
  /** Tells the viewer why a request failed that was sent for `revision`. */
  fail(error: unknown, revision: number): void;
  /** Restarts the servers of `language`; resolves with why they did not start, if so. */
  restartServer(language: string): Promise<string | undefined>;
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

  /** Restarts the server of `language`, then asks again; a server that does not start is told, not asked. */
  private async restart(search: Search, language: string, shown: Omit<PickerView, "body">): Promise<void> {
    this.running?.abort.abort();
    const mine = { abort: new AbortController() };
    this.running = mine;
    this.deps.picker.show({ ...shown, body: { phase: "searching" } });
    const problem = await this.deps.restartServer(language);
    if (this.running !== mine) return;
    if (problem !== undefined) {
      this.running = undefined;
      this.deps.picker.close();
      this.deps.notice(`Cannot restart the ${language} language server: ${problem}`);
      return;
    }
    await this.start(search);
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
        search.open(row);
      },
      onretry: () => void this.start(search),
      onclose: () => this.cancel(),
    };
    const timer = new Debounced();
    timer.schedule(this.delayMs, () => {
      if (current()) picker.show({ ...shown, body: { phase: "searching" } });
    });
    let answer: LocationRows;
    try {
      answer = await search.ask(mine.abort.signal);
    } catch (error) {
      timer.cancel();
      if (!current()) return;
      this.running = undefined;
      picker.close();
      if (this.deps.isAbort(error)) this.deps.notice(STALE_NOTICE);
      else this.deps.fail(error, search.revision);
      return;
    }
    timer.cancel();
    if (!current()) return;
    const { unavailable, rows, total } = answer;
    if (unavailable?.kind === "serverMissing" || unavailable?.kind === "serverFailed") {
      const { language } = unavailable;
      const onretry = language === null ? shown.onretry : () => void this.restart(search, language, shown);
      picker.show({ ...shown, onretry, body: { phase: "stopped", message: unavailable.message } });
      return;
    }
    this.running = undefined;
    if (unavailable) {
      picker.close();
      this.deps.notice(unavailable.message);
    } else if (total === 1 && rows.length === 1) {
      picker.close();
      search.open(rows[0]);
    } else {
      const { keep } = search;
      const onkeep =
        keep &&
        (() => {
          this.running = undefined;
          picker.close();
          keep(answer);
        });
      picker.show({ ...shown, onkeep, body: { phase: "results", rows, total } });
    }
  }
}
