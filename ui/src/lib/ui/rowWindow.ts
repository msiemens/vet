// Which rows of a fixed-height list to render: the ones in the viewport plus `overscan` on each side.

export interface RowWindow {
  start: number;
  end: number;
}

/** The half-open range `[start, end)` of rows to mount for a viewport of `viewport` px scrolled by `scrollTop`. */
export function rowWindow(
  scrollTop: number,
  viewport: number,
  rowHeight: number,
  count: number,
  overscan: number,
): RowWindow {
  if (count <= 0 || rowHeight <= 0) return { start: 0, end: 0 };
  const first = Math.floor(Math.max(0, scrollTop) / rowHeight);
  const last = Math.ceil((Math.max(0, scrollTop) + Math.max(0, viewport)) / rowHeight);
  return { start: Math.min(count, Math.max(0, first - overscan)), end: Math.min(count, last + overscan) };
}

/** The scroll offset that brings row `index` fully into view with the least movement, or `scrollTop` if it already is. */
export function scrollToRow(index: number, scrollTop: number, viewport: number, rowHeight: number): number {
  const top = index * rowHeight;
  if (top < scrollTop) return top;
  if (top + rowHeight > scrollTop + viewport) return top + rowHeight - viewport;
  return scrollTop;
}
