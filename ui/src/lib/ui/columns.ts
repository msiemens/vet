// The columns of a `Table`: their widths as the viewer set them, the grid they lay out, and which
// edge of the header resizes which column.

export interface Column<R = unknown> {
  id: string;
  /** The header's text; empty for a column of icons. */
  label: string;
  /** What the column holds, for the resize handle's name when it differs from the label. */
  name?: string;
  /** The width in px until the viewer resizes the column; a flexible column has none. */
  width?: number;
  /** The smallest width in px the viewer can give the column. */
  min?: number;
  /** Takes the room the other columns leave. At most one column of a table is flexible. */
  flex?: boolean;
  /** Keeps its width: the viewer cannot resize it. */
  fixed?: boolean;
  align?: "right";
  /**
   * Makes the column sortable: what a row sorts by in it, text by the locale or a number. A row
   * without one comes last in either direction.
   */
  sortBy?: (row: R) => string | number | undefined;
  /** The direction of the first click on a sortable column's header. */
  firstSort?: "ascending" | "descending";
}

/** The widths the viewer set, in px by column id; a column not in it has its default. */
export type Widths = Readonly<Partial<Record<string, number>>>;

const DEFAULT_MIN = 40;

/** A column of any table, as far as its layout goes. */
type Sized = Omit<Column, "sortBy">;

export const minOf = (column: Sized) => column.min ?? DEFAULT_MIN;

const resizable = (column: Sized) => !column.flex && !column.fixed;

/** The `grid-template-columns` of the header and the rows. */
export function gridTemplate(columns: readonly Sized[], widths: Widths): string {
  return columns.map((c) => (c.flex ? "minmax(0, 1fr)" : `${widths[c.id] ?? c.width ?? minOf(c)}px`)).join(" ");
}

/**
 * Which edge of a column's header cell resizes it, if any. A column left of the flexible one is
 * resized by its right edge and one right of it by its left edge, so the edge follows the pointer
 * while the flexible column takes up the difference.
 */
export function handleSide(columns: readonly Sized[], index: number): "start" | "end" | undefined {
  if (!resizable(columns[index])) return undefined;
  const flex = columns.findIndex((c) => c.flex);
  return flex < 0 || index < flex ? "end" : "start";
}

/**
 * The width a column takes when its handle moves by `delta` px (rightwards positive) from where a
 * drag started: at least the column's minimum, and at most what the flexible column can give
 * (`room`: how far it is above its own minimum).
 */
export function resized(column: Sized, side: "start" | "end", start: number, delta: number, room = Infinity): number {
  const wanted = start + (side === "end" ? delta : -delta);
  return Math.round(Math.max(minOf(column), Math.min(start + Math.max(0, room), wanted)));
}

export function withWidth(widths: Widths, id: string, width: number): Widths {
  return { ...widths, [id]: width };
}

/** Puts a column back to its default width. */
export function withoutWidth(widths: Widths, id: string): Widths {
  return Object.fromEntries(Object.entries(widths).filter(([key]) => key !== id));
}

/** The text to store, or none when every column has its default. */
export function serializeWidths(widths: Widths): string | undefined {
  return Object.keys(widths).length ? JSON.stringify(widths) : undefined;
}

/** Stored widths for these columns: others are dropped, and none is narrower than its minimum. */
export function parseWidths(columns: readonly Sized[], text: string | undefined): Widths {
  if (text === undefined) return {};
  let value: unknown;
  try {
    value = JSON.parse(text);
  } catch {
    return {};
  }
  if (typeof value !== "object" || value === null) return {};
  const stored = value as Record<string, unknown>;
  const widths: Record<string, number> = {};
  for (const column of columns) {
    const width = stored[column.id];
    if (resizable(column) && typeof width === "number" && Number.isFinite(width)) {
      widths[column.id] = Math.max(minOf(column), Math.round(width));
    }
  }
  return widths;
}
