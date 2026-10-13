/** The smallest height the dock is dragged to, and the room the review above it keeps. */
export const DOCK_MIN = 120;
export const DOCK_KEPT_ABOVE = 140;

/** The smallest and largest height the dock takes in a review area `area` px high. */
export function dockLimits(area: number): [number, number] {
  return [DOCK_MIN, Math.max(DOCK_MIN, area - DOCK_KEPT_ABOVE)];
}
