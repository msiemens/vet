// Navigation between the hunks flagged for later review, across the files of a view.

/** A hunk of a file; `hunk` is -1 for a place before the file's first hunk (a change card). */
export interface HunkPlace {
  path: string;
  hunk: number;
}

/**
 * The flagged hunk after (`delta` 1) or before (-1) `from` over the files `paths` in order, without
 * wrapping around; with no `from` the first or the last one. `flagged` says which hunks of a file are
 * flagged now. Undefined when there is none.
 */
export function stepFlagged(
  paths: readonly string[],
  flagged: (path: string) => readonly boolean[] | undefined,
  from: HunkPlace | undefined,
  delta: 1 | -1,
): HunkPlace | undefined {
  const places = paths.flatMap((path) => (flagged(path) ?? []).flatMap((on, hunk) => (on ? [{ path, hunk }] : [])));
  if (delta === 1) return places.find((p) => from === undefined || after(paths, p, from));
  return places.reverse().find((p) => from === undefined || after(paths, from, p));
}

/** Whether `a` lies after `b` in the order of `paths`; a place is not after itself. */
function after(paths: readonly string[], a: HunkPlace, b: HunkPlace): boolean {
  const [ia, ib] = [paths.indexOf(a.path), paths.indexOf(b.path)];
  return ia !== ib ? ia > ib : a.hunk > b.hunk;
}
