import type { BaseDiff } from "../../../generated/BaseDiff.ts";

/** "base a41c9e2 → c03d55f"; the old base is unnamed once git has dropped its commit. */
export function baseDiffTitle(diff: BaseDiff): string {
  return diff.from ? `base ${diff.from.shortId} → ${diff.to.shortId}` : `base → ${diff.to.shortId}`;
}
