/** How many avatar colours the palette (`--avatar-1` …) has. */
const COLOURS = 6;

/** Which palette colour (1-based) a name or path gets: the same every time, whatever else is listed. */
export function avatarColor(key: string): number {
  let hash = 0;
  for (const char of key) hash = (Math.imul(hash, 31) + (char.codePointAt(0) ?? 0)) >>> 0;
  return (hash % COLOURS) + 1;
}
