// When the checks of a commit with a running check are asked for again.

/** How long a commit with a running check waits before it is asked about again. */
export const REFRESH_MS = 30_000;

/** How long before a commit with a running check is asked about again: not before a limit ends. */
export function refreshDelay(limitedUntil: number | undefined, nowMs: number): number {
  return Math.max(REFRESH_MS, limitedUntil === undefined ? 0 : limitedUntil * 1000 - nowMs);
}
