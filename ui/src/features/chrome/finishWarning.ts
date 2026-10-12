// What Finish Review says about the hunks that are still flagged for later review.

/** The warning for `flagged` hunks still flagged; none when there are none. */
export function flaggedWarning(flagged: number): string | undefined {
  if (flagged <= 0) return undefined;
  return flagged === 1
    ? "1 hunk is still flagged for later review."
    : `${flagged} hunks are still flagged for later review.`;
}
