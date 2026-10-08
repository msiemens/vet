// Which words differ between the two texts of a comment that was edited in two places, so the
// conflict card can mark them.

export interface Piece {
  text: string;
  /** Not in the other text. */
  changed: boolean;
}

/** Words and the white space between them, so joining the pieces gives the text back. */
const tokens = (text: string) => text.match(/\s+|[^\s]+/g) ?? [];

/** `text` in pieces, those with a word the `other` text does not have at that place marked. */
export function markDifferences(text: string, other: string): Piece[] {
  const a = tokens(text);
  const b = tokens(other);
  // Longest common subsequence, by length of the tails.
  const tail = Array.from({ length: a.length + 1 }, () => new Array<number>(b.length + 1).fill(0));
  for (let i = a.length - 1; i >= 0; i--)
    for (let j = b.length - 1; j >= 0; j--)
      tail[i][j] = a[i] === b[j] ? tail[i + 1][j + 1] + 1 : Math.max(tail[i + 1][j], tail[i][j + 1]);

  const pieces: Piece[] = [];
  const add = (token: string, changed: boolean) => {
    const last = pieces.at(-1);
    if (last?.changed === changed) last.text += token;
    else pieces.push({ text: token, changed });
  };
  let [i, j] = [0, 0];
  while (i < a.length) {
    if (j < b.length && a[i] === b[j]) {
      add(a[i++], false);
      j++;
    } else if (j < b.length && tail[i][j + 1] >= tail[i + 1][j]) j++;
    else {
      // White space alone is never the difference.
      add(a[i], !/^\s+$/.test(a[i]));
      i++;
    }
  }
  return pieces;
}
