// How the UI writes counts, nouns, times of day, durations and compares text. One locale for
// all of it, so a number or a sort reads the same on every screen.

const LOCALE = "en";

const numbers = new Intl.NumberFormat(LOCALE);
const pluralRules = new Intl.PluralRules(LOCALE);

/** A number with its thousands separators: "1,234". */
export const count = (n: number): string => numbers.format(n);

/** `one` for 1, else `many` (the plural by an added `s` unless given). */
export const noun = (n: number, one: string, many = `${one}s`): string =>
  pluralRules.select(n) === "one" ? one : many;

/** A count with its noun: "1 file", "2 files". */
export const plural = (n: number, one: string, many?: string): string => `${count(n)} ${noun(n, one, many)}`;

/** A text collator; `localeCompare` creates one per call, so sorts hold one of these. */
export const collator = (options?: Intl.CollatorOptions): Intl.Collator => new Intl.Collator(LOCALE, options);

/** The time of day is always 24-hour, as the design writes it ("Mon 14:02", "09:40"). */
export const TIME_OF_DAY = { hour: "2-digit", minute: "2-digit", hourCycle: "h23" } as const;

const minutes = new Intl.DateTimeFormat(LOCALE, TIME_OF_DAY);
const seconds = new Intl.DateTimeFormat(LOCALE, { ...TIME_OF_DAY, second: "2-digit" });

/** The time of day of a unix-seconds instant: "14:02", or "14:02:09" with `withSeconds`. */
export const clock = (unixSeconds: number, withSeconds = false): string =>
  (withSeconds ? seconds : minutes).format(unixSeconds * 1000);

/** A run's duration as `4m 40s`, `12s` or `1h 02m`. */
export function durationText(totalSeconds: number): string {
  const mins = Math.floor(totalSeconds / 60);
  if (totalSeconds < 60) return `${totalSeconds}s`;
  if (mins < 60) return `${mins}m ${String(totalSeconds % 60).padStart(2, "0")}s`;
  return `${Math.floor(mins / 60)}h ${String(mins % 60).padStart(2, "0")}m`;
}
