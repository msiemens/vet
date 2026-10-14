// How a preference is written as the JSON text the backend keeps, and read back.

/**
 * The value kept as `text`. `parse` turns the JSON into a value, or into `undefined` for one that
 * is not a value of the preference; that, damaged JSON and no text all give the `fallback`.
 */
export function decode<T>(text: string | undefined, parse: (json: unknown) => T | undefined, fallback: T): T {
  if (text === undefined) return fallback;
  try {
    return parse(JSON.parse(text) as unknown) ?? fallback;
  } catch {
    return fallback;
  }
}

/** The text to keep for `value`; none for the `fallback`, so that a kept value never outlives a change of the default. */
export function encode<T>(value: T, fallback: T): string | undefined {
  const text = JSON.stringify(value) as string | undefined;
  return text === (JSON.stringify(fallback) as string | undefined) ? undefined : text;
}
