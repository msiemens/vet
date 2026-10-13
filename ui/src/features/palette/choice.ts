import type { SymbolChange } from "../../generated/SymbolChange";
import type { SymbolInfo } from "../../generated/SymbolInfo";

/** What the viewer chose; the owner makes it happen once the popup has gone and the focus is back. */
export type Choice =
  /** `line` is 0-based; `inChanges` shows the file in All changes instead of opening its tab. */
  | { type: "file"; path: string; line?: number; inChanges: boolean }
  /** `reveal` selects a changed symbol where it is reviewed. */
  | { type: "symbol"; symbol: SymbolInfo; change?: SymbolChange; reveal: boolean }
  | { type: "changeset"; id: string }
  | { type: "thread"; id: number }
  | { type: "action"; id: string };
