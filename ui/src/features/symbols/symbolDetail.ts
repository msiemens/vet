// Pure logic of the symbol details: the words shown for a symbol and for the fate of a call site.

import type { CallFate } from "../../generated/CallFate";
import type { Position } from "../../generated/Position";
import type { Side } from "../../generated/Side";
import type { SymbolId } from "../../generated/SymbolId";
import type { SymbolName } from "../../generated/SymbolName";
import { containersOf, fullName, separatorOf } from "../../lib/symbolLabel.ts";

/** Opens a file at a 0-based `line` of `side`; no line opens it where it is. */
export type OpenLocation = (path: string, line: number | undefined, side: Side) => void;

/**
 * What the panel's head says of where a symbol is: the types around it, outermost first, as `A::B` or
 * `A.B`, and the namespaces or modules above them; each empty when there are none.
 */
export function headOf({ id, modules }: SymbolName): { container: string; namespace: string } {
  const names = containersOf(id);
  const join = (parts: string[]) => parts.join(separatorOf(id.path));
  return { namespace: join(names.slice(0, modules)), container: join(names.slice(modules)) };
}

/** `path:line` with a 1-based line, as editors print it. */
export const place = (path: string, at: Position): string => `${path}:${at.line + 1}`;

/** What a row's tooltip says: the symbol's whole name, if there is one, then where it is. */
export const rowTitle = (id: SymbolId | null, path: string, at?: Position): string =>
  [id ? fullName(id) : undefined, at ? place(path, at) : path].filter(Boolean).join("\n");

export type FateTone = "bad" | "warn" | "mute";

/** What became of a reference to a removed symbol, in words; `likelyBroken` is the one that needs attention. */
export function describeFate(fate: CallFate): { label: string; tone: FateTone; detail?: string } {
  switch (fate.type) {
    case "gone":
      return { label: "deleted", tone: "mute" };
    case "modified":
      return { label: "edited", tone: "mute" };
    case "likelyBroken":
      return { label: "likely broken", tone: "bad" };
    case "bindingChanged":
      return {
        label: "binds elsewhere now",
        tone: "warn",
        detail: fate.now.map((l) => place(l.path, l.span.start)).join(", ") || undefined,
      };
    case "unknown":
      return { label: "unknown", tone: "mute", detail: fate.reason };
  }
}
