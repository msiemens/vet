const weekday = new Intl.DateTimeFormat("en", { weekday: "short" });
const monthDay = new Intl.DateTimeFormat("en", { month: "short", day: "numeric" });
const monthDayYear = new Intl.DateTimeFormat("en", { month: "short", day: "numeric", year: "numeric" });
const clock = new Intl.DateTimeFormat("en", { hour: "numeric", minute: "2-digit" });

const startOfDay = (ms: number) => new Date(ms).setHours(0, 0, 0, 0);

/**
 * When something happened, as short as a list row needs: "now" for the last minute, the time of
 * day today, the weekday within the last week, then the date (with the year when it is not this one).
 */
export function shortTime(unixSeconds: number, nowMs = Date.now()): string {
  const at = unixSeconds * 1000;
  if (nowMs - at < 60_000) return "now";
  const days = Math.round((startOfDay(nowMs) - startOfDay(at)) / 86_400_000);
  if (days <= 0) return clock.format(at);
  if (days < 7) return weekday.format(at);
  return (new Date(at).getFullYear() === new Date(nowMs).getFullYear() ? monthDay : monthDayYear).format(at);
}
