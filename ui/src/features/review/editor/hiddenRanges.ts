// Analysis hints (diagnostics) are drawn only where the editor shows text: a region collapsed behind
// an "N unchanged lines" bar shows none, and neither does the bar. This is the pure part: which
// pieces of a range lie outside the hidden regions.

/** A document range `[from, to]`, positions in UTF-16 units. */
export interface Span {
  from: number;
  to: number;
}

/**
 * The parts of `span` that are not inside `hidden` (closed ranges, in any order, that do not
 * overlap): none if all of it is hidden, several if a hidden region lies in the middle. A range
 * of no length survives unless it sits inside a hidden one.
 */
export function outsideHidden(span: Span, hidden: readonly Span[]): Span[] {
  const regions = [...hidden].sort((a, b) => a.from - b.from);
  if (span.from === span.to) return regions.some((h) => h.from <= span.from && span.from <= h.to) ? [] : [span];
  const pieces: Span[] = [];
  let at = span.from;
  for (const h of regions) {
    if (h.to < at) continue;
    if (h.from > span.to) break;
    if (h.from - 1 > at) pieces.push({ from: at, to: h.from - 1 });
    at = Math.max(at, h.to + 1);
  }
  if (span.to > at) pieces.push({ from: at, to: span.to });
  return pieces;
}
