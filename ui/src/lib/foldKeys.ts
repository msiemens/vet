// What ← and → do in a tree of rows that open and shut, and the set of folded keys they change.

/** What a row has to do with ← and →: how deep it sits. */
export interface Foldable {
  depth: number;
}

/** How a tree's rows answer ← and →. */
export interface Folding<R> {
  /** Whether a row can be open or shut. */
  holds: (row: R) => boolean;
  /** Whether a row that holds rows is open. */
  open: (row: R) => boolean;
  /** Whether ← on a row that is not shut open steps to the row that holds it. */
  leaves: (row: R) => boolean;
}

/**
 * What ← or → does on the row at `index`: opens or shuts it, or moves the highlight, to the first
 * row inside it or to the row that holds it.
 */
export function arrowKey<R extends Foldable>(
  rows: readonly R[],
  index: number,
  key: "left" | "right",
  { holds, open, leaves }: Folding<R>,
): { toggle: R } | { move: number } | undefined {
  if (index < 0 || index >= rows.length) return undefined;
  const row = rows[index];
  if (key === "right") {
    if (!holds(row)) return undefined;
    if (!open(row)) return { toggle: row };
    return index + 1 < rows.length && rows[index + 1].depth > row.depth ? { move: index + 1 } : undefined;
  }
  if (holds(row) && open(row)) return { toggle: row };
  if (!leaves(row)) return undefined;
  for (let at = index - 1; at >= 0; at--) if (rows[at].depth < row.depth) return { move: at };
  return undefined;
}

/** Adds `key` to `set`, or removes it if it was in: the fold of a row, the open state of a card. */
export function toggleIn<T>(set: Set<T>, key: T): void {
  if (!set.delete(key)) set.add(key);
}

/** A copy of `set` with `key` toggled, for state that is replaced rather than changed. */
export function toggled<T>(set: ReadonlySet<T>, key: T): Set<T> {
  const next = new Set(set);
  toggleIn(next, key);
  return next;
}
