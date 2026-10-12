import { shortTime } from "./shortTime.ts";

const fullFormat = new Intl.DateTimeFormat(undefined, {
  weekday: "short",
  day: "numeric",
  month: "short",
  year: "numeric",
  hour: "numeric",
  minute: "2-digit",
});

/**
 * How long ago something happened, as a comment shows it: "now" for the last minute, minutes, hours,
 * "yesterday", days within the week, then the date `shortTime` gives.
 */
export function relativeTime(unixSeconds: number, nowMs = Date.now()): string {
  const ago = nowMs - unixSeconds * 1000;
  if (ago < 60_000) return "now";
  const minutes = Math.floor(ago / 60_000);
  if (minutes < 60) return `${minutes} min ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours} h ago`;
  const days = Math.floor(hours / 24);
  if (days === 1) return "yesterday";
  if (days < 7) return `${days} days ago`;
  return shortTime(unixSeconds, nowMs);
}

/** The full date and time, for the tooltip of a relative one. */
export const fullTime = (unixSeconds: number): string => fullFormat.format(unixSeconds * 1000);
