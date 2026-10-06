/** A 0-based line range as the viewer reads it: `L12` or `L12–14`. */
export function lineRange(start: number, len: number): string {
  return len <= 1 ? `L${start + 1}` : `L${start + 1}–${start + len}`;
}
