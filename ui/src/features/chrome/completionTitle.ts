import type { Completion } from "../../generated/Completion.ts";

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
