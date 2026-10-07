// What a change card says, as data. `changeCard` draws it and `sectionLayout` counts its
// rows from it, so the card and the height reserved for it cannot disagree.

import type { EntryDesc } from "../../../generated/EntryDesc";
import type { NonTextItem } from "../../../generated/NonTextItem";
import type { NonTextKind } from "../../../generated/NonTextKind";
import { formatBytes } from "../../../lib/formatBytes.ts";

export type CardRow =
  /** One side of the change. */
  | { type: "change"; sign: "−" | "+"; text: string }
  /** What to say when there is nothing to compare. */
  | { type: "note"; text: string };

export interface CardContent {
  title: string;
  rows: CardRow[];
}

const TITLE: Record<NonTextKind, string> = {
  mode: "File mode",
  symlink: "Symlink target",
  submodule: "Submodule commit",
  binary: "Binary file",
  contentUnavailable: "Content unavailable",
  rename: "Renamed",
  empty: "Empty file",
};

const size = (bytes: number | null): string => (bytes === null ? "" : formatBytes(bytes));

/** One side of the change as text. */
function describe(kind: NonTextKind, side: EntryDesc | null, path: string): string {
  if (!side) return "(none)";
  switch (kind) {
    case "mode":
      return side.mode.toString(8);
    case "symlink":
      return side.target ?? "";
    case "submodule":
      return side.target?.slice(0, 7) ?? "";
    case "binary":
    case "empty":
      return size(side.size);
    case "rename":
      return path;
    default:
      return "";
  }
}

export function cardContent(item: NonTextItem, path: string, oldPath: string | null): CardContent {
  const title = TITLE[item.kind];
  if (item.kind === "contentUnavailable")
    return { title, rows: [{ type: "note", text: "The content of this file is not available." }] };
  return {
    title,
    rows: [
      {
        type: "change",
        sign: "−",
        text: item.kind === "rename" ? (oldPath ?? "") : describe(item.kind, item.before, path),
      },
      { type: "change", sign: "+", text: describe(item.kind, item.after, path) },
    ],
  };
}
