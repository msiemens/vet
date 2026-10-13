import type { Completion } from "../../generated/Completion.ts";

/** There is no changed line or other change to see, so a percentage would say nothing. */
export const nothingToReview = (c: Completion): boolean => c.lines + c.items === 0;

/** The toolbar meter's tooltip: the changed lines by state, then the non-text changes; base changed only when some are. */
export function completionTitle(c: Completion): string {
  const unseen = c.lines - c.seenLines - c.oldBaseLines;
  return [
    `${c.seenLines} seen`,
    ...(c.oldBaseLines > 0 ? [`${c.oldBaseLines} base changed`] : []),
    `${unseen} unseen`,
    `${c.acknowledgedItems} of ${c.items} other changes`,
  ].join(" · ");
}
