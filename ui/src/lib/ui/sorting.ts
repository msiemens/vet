// The order of a `Table`'s rows: by one sortable column, either way, or the order its owner gives
// them in (the default). Rows that sort alike keep the default order between them.

import { collator } from "../format.ts";
import type { Column } from "./columns";

export type Direction = "ascending" | "descending";

export interface Sort {
  column: string;
  direction: Direction;
}

const opposite = (direction: Direction): Direction => (direction === "ascending" ? "descending" : "ascending");

/** The sort after a click on a column's header: its first direction, then the other, then the default order. */
export function nextSort<R>(sort: Sort | undefined, column: Column<R>): Sort | undefined {
  const first = column.firstSort ?? "ascending";
  if (sort?.column !== column.id) return { column: column.id, direction: first };
  return sort.direction === first ? { column: column.id, direction: opposite(first) } : undefined;
}

const text = collator({ numeric: true });

function compare(a: string | number, b: string | number): number {
  if (typeof a === "number" && typeof b === "number") return a - b;
  return text.compare(String(a), String(b));
}

/** The rows in the order of `sort`; without one, or by a column that cannot sort, as given. */
export function sortRows<R>(
  rows: readonly R[],
  columns: readonly Column<NoInfer<R>>[],
  sort: Sort | undefined,
): readonly R[] {
  const by = sort && columns.find((c) => c.id === sort.column)?.sortBy;
  if (!sort || !by) return rows;
  const sign = sort.direction === "ascending" ? 1 : -1;
  return rows
    .map((row) => ({ row, value: by(row) }))
    .sort((a, b) => {
      if (a.value === undefined || b.value === undefined) {
        return (a.value === undefined ? 1 : 0) - (b.value === undefined ? 1 : 0);
      }
      return sign * compare(a.value, b.value);
    })
    .map(({ row }) => row);
}

/** The text to store, or none for the default order. */
export function serializeSort(sort: Sort | undefined): string | undefined {
  return sort && JSON.stringify(sort);
}

/** A stored sort, if it names a sortable column of these. */
export function parseSort<R>(columns: readonly Column<R>[], value: unknown): Sort | undefined {
  if (typeof value !== "object" || value === null) return undefined;
  const { column, direction } = value as Record<string, unknown>;
  if (direction !== "ascending" && direction !== "descending") return undefined;
  return columns.some((c) => c.id === column && c.sortBy) ? { column: column as string, direction } : undefined;
}
