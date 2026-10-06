const UNITS: [Intl.RelativeTimeFormatUnit, number][] = [
  ["year", 365 * 86400],
  ["month", 30 * 86400],
  ["week", 7 * 86400],
  ["day", 86400],
  ["hour", 3600],
  ["minute", 60],
];

const format = new Intl.RelativeTimeFormat("en", { numeric: "always", style: "short" });

/** "3 days ago" for a Unix time in seconds; anything under a minute is "just now". */
export function relativeTime(unixSeconds: number, nowMs = Date.now()): string {
  const diff = unixSeconds - nowMs / 1000;
  for (const [unit, seconds] of UNITS) {
    if (Math.abs(diff) >= seconds) return format.format(Math.round(diff / seconds), unit);
  }
  return "just now";
}
