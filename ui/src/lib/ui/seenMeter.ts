/** A stacked meter's segment: `value` in percent, `color` any CSS colour. */
export interface Segment {
  value: number;
  color: string;
}

/** What is seen in blue, then what is seen only against an old base in its stripes (never solid). */
export const seenSegments = (seen: number, old: number): Segment[] => [
  { value: seen, color: "var(--seen)" },
  { value: old, color: "var(--old-base-stripes)" },
];

/**
 * A diffstat bar: green and red parts in the ratio of added to removed lines (neutral when there are
 * none), covered from the left by what is seen, so a fully reviewed bar is blue.
 */
export function diffSegments(added: number, removed: number, seen: number, old: number): Segment[] {
  const total = added + removed;
  const addEnd = total ? (added / total) * 100 : 0;
  const covered = Math.min(100, seen + old);
  return [
    ...seenSegments(seen, old),
    { value: Math.max(0, addEnd - covered), color: "var(--add-text)" },
    { value: total ? Math.min(100 - addEnd, 100 - covered) : 0, color: "var(--del-text)" },
  ];
}
