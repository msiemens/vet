/** The tab `delta` places from `active` among the tabs in display order, wrapping around the ends. */
export function tabAfter<T>(open: readonly T[], active: T, delta: number): T {
  const at = open.indexOf(active);
  return open[(((at + delta) % open.length) + open.length) % open.length];
}

/**
 * The tabs after `from` gives way to `to`: `to` takes `from`'s place, or, when it is open already,
 * `from` just closes. Never lists a tab twice.
 */
export function swapTab<T>(open: readonly T[], from: T, to: T): T[] {
  if (open.includes(to)) return open.filter((tab) => tab !== from);
  return open.map((tab) => (tab === from ? to : tab));
}
