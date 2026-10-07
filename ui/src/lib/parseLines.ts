/** The entries of a one-per-line field: trimmed, blank lines dropped. */
export function parseLines(text: string): string[] {
  return text
    .split("\n")
    .map((line) => line.trim())
    .filter(Boolean);
}
