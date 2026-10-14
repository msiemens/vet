import type { LineDelta } from "../../generated/LineDelta.ts";

/** The signed counts of a change, with the true minus sign: `["+12", "−3"]`. */
export const deltaParts = (added: number, removed: number): [string, string] => [`+${added}`, `−${removed}`];

/** The counts as one line of text, for a tooltip: `+12 −3`. */
export const deltaText = (added: number, removed: number): string => deltaParts(added, removed).join(" ");

/** The changes of several things taken together. */
export const sumDelta = (deltas: Iterable<LineDelta>): LineDelta => {
  const total = { added: 0, removed: 0 };
  for (const d of deltas) {
    total.added += d.added;
    total.removed += d.removed;
  }
  return total;
};
