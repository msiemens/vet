// When the Comments tool window of each pull request was last open, as one preference: the most
// recently written times, so that it does not grow with every pull request ever viewed.

/** Unix seconds by `seenKey`, the least recently written first. */
export type SeenTimes = Readonly<Record<string, number>>;

/** The most pull requests whose time is kept. */
export const MAX_SEEN = 500;

/** The times kept as `value`; whatever is not a time is dropped. */
export function parseSeen(value: unknown): SeenTimes {
  if (typeof value !== "object" || value === null || Array.isArray(value)) return {};
  return Object.fromEntries(
    Object.entries(value as Record<string, unknown>).filter(
      (entry): entry is [string, number] => typeof entry[1] === "number" && Number.isFinite(entry[1]) && entry[1] > 0,
    ),
  );
}

/** `seen` with `key` last written at `at`, and the oldest dropped beyond `max`. */
export function withSeen(seen: SeenTimes, key: string, at: number, max = MAX_SEEN): SeenTimes {
  const kept = [...Object.entries(seen).filter(([other]) => other !== key), [key, at] as const];
  return Object.fromEntries(kept.slice(Math.max(0, kept.length - max)));
}
