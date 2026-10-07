import type { Completion } from "../generated/Completion";

/** How much of a changeset or file is seen, and how much of that is seen against an old base, in percent. */
export function percentages(c: Completion): { seenPct: number; oldPct: number } {
  const total = c.lines + c.items;
  const pct = (n: number) => (total ? (n / total) * 100 : 0);
  return { seenPct: pct(c.seenLines + c.acknowledgedItems), oldPct: pct(c.oldBaseLines) };
}
