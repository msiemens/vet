/** The tab `delta` places from `active` among the tabs in display order, wrapping around the ends. */
export function tabAfter<T>(open: readonly T[], active: T, delta: number): T {
  const at = open.indexOf(active);
  return open[(((at + delta) % open.length) + open.length) % open.length];
}
