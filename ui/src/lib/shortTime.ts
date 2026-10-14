import { TIME_OF_DAY, clock } from "./format.ts";

const weekday = new Intl.DateTimeFormat("en", { weekday: "short" });
const weekdayClock = new Intl.DateTimeFormat("en", { weekday: "short", ...TIME_OF_DAY });
const monthDay = new Intl.DateTimeFormat("en", { month: "short", day: "numeric" });
const monthDayYear = new Intl.DateTimeFormat("en", { month: "short", day: "numeric", year: "numeric" });
const fullFormat = new Intl.DateTimeFormat(undefined, {
  weekday: "short",
  day: "numeric",
  month: "short",
  year: "numeric",
  ...TIME_OF_DAY,
});

const startOfDay = (ms: number) => new Date(ms).setHours(0, 0, 0, 0);

/**
 * `compact` is for a narrow column (the queue's Updated, the recent list): the weekday alone within
 * the last week. `detailed` is for text with room (comments, commits): the weekday carries the time of day.
 */
export type TimeStyle = "compact" | "detailed";

/**
 * When something happened, as short as a row needs: "now" for the last minute, the time of day
 * today, the weekday within the last week, then the date (with the year when it is not this one).
 */
export function shortTime(unixSeconds: number, nowMs: number = Date.now(), style: TimeStyle = "compact"): string {
  const at = unixSeconds * 1000;
  if (nowMs - at < 60_000) return "now";
  const days = Math.round((startOfDay(nowMs) - startOfDay(at)) / 86_400_000);
  if (days <= 0) return clock(unixSeconds);
  if (days < 7) return (style === "detailed" ? weekdayClock : weekday).format(at);
  return (new Date(at).getFullYear() === new Date(nowMs).getFullYear() ? monthDay : monthDayYear).format(at);
}

/** The full date and time, for the tooltip of a short one. */
export const fullTime = (unixSeconds: number): string => fullFormat.format(unixSeconds * 1000);
