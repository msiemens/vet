// Browser storage for small per-viewer conveniences. It may be unavailable (a private window,
// blocked site data); what is written then lasts for this session only.

export function readStored(key: string): string | undefined {
  try {
    return localStorage.getItem(key) ?? undefined;
  } catch {
    return undefined;
  }
}

/** `undefined` forgets the value. */
export function writeStored(key: string, value: string | undefined): void {
  try {
    if (value === undefined) localStorage.removeItem(key);
    else localStorage.setItem(key, value);
  } catch {
    // Not remembered.
  }
}

/** The JSON in `text`; none when there is no text or it is damaged. */
export function parseJson(text: string | undefined): unknown {
  if (text === undefined) return undefined;
  try {
    return JSON.parse(text);
  } catch {
    return undefined;
  }
}

/** What `validate` makes of the JSON stored under `key`; it answers `undefined` for a value that is not one of its own. */
export function readJson<T>(key: string, validate: (value: unknown) => T | undefined): T | undefined {
  const value = parseJson(readStored(key));
  return value === undefined ? undefined : validate(value);
}

/** The older spellings of on and off are read too, so a setting kept by an earlier version holds. */
const FLAGS = new Map([
  ["1", true],
  ["open", true],
  ["0", false],
  ["shut", false],
]);

/** An on/off setting; `fallback` while none is kept or the kept value is not an on or off. */
export const storedFlag = (key: string, fallback: boolean): boolean => FLAGS.get(readStored(key) ?? "") ?? fallback;

/** Keeps an on/off setting; the default is not kept, so it follows a later change of it. */
export function storeFlag(key: string, on: boolean, fallback: boolean): void {
  writeStored(key, on === fallback ? undefined : on ? "1" : "0");
}
