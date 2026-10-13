// What ← and → do in a tree of rows that open and shut.

/** What a row has to do with ← and →: whether it can be open or shut, and whether it is open. */
export interface Foldable {
  kind: string;
  depth: number;
}

/**
 * What ← or → does on the row at `index`: opens or shuts it, or moves the highlight, to the first
 * row inside it or to the row that holds it. `holds` says whether a row can be open or shut and
 * `open` whether it is.
 */
export function arrowKey<R extends Foldable>(
  rows: readonly R[],
  index: number,
  key: "left" | "right",
  holds: (row: R) => boolean,
  open: (row: R) => boolean,
): { toggle: R } | { move: number } | undefined {
  if (index < 0 || index >= rows.length) return undefined;
  const row = rows[index];
  if (key === "right") {
    if (!holds(row)) return undefined;
    if (!open(row)) return { toggle: row };
    return index + 1 < rows.length && rows[index + 1].depth > row.depth ? { move: index + 1 } : undefined;
  }
  if (holds(row) && open(row)) return { toggle: row };
  if (row.kind !== "symbol") return undefined;
  for (let at = index - 1; at >= 0; at--) if (rows[at].depth < row.depth) return { move: at };
  return undefined;
}
