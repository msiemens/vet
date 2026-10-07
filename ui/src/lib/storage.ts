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
