// What the location picker shows and is asked: types only, shared by the search, the store and the
// editors that start a search.

import type { LocationRow } from "../generated/LocationRow";
import type { Side } from "../generated/Side";

export type PickerKind = "usages" | "implementations";

/** What the popup says of the search. */
export type PickerBody =
  /** The server has not answered yet. */
  | { phase: "searching" }
  /** The answer: `total` is more than `rows` has when the answer was cut. */
  | { phase: "results"; rows: readonly LocationRow[]; total: number }
  /** The language server cannot answer; `message` says why. */
  | { phase: "stopped"; message: string };

/** The caret's place in the window: where the popup opens, below its line or above when there is no room. */
export interface CaretRect {
  left: number;
  top: number;
  bottom: number;
}

export interface PickerView {
  kind: PickerKind;
  /** The name at the caret, which the title shows in the code font. */
  symbol: string;
  anchor: CaretRect;
  /** Which text the rows' positions are in. */
  side: Side;
  body: PickerBody;
  /** Opens the row's place. */
  onpick(row: LocationRow): void;
  /** Asks again, for a server that was not running. */
  onretry(): void;
  /** The popup was closed without a pick. */
  onclose(): void;
}

/** What `LocationSearch` needs of the popup. */
export interface PickerSink {
  show(view: PickerView): void;
  close(): void;
}
