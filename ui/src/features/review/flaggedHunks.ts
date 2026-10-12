// Navigation between the hunks flagged for later review, across the files of a view.

/** The files of a view in the order they are shown, with what navigation needs of their review state. */
export interface FlagFile {
  path: string;
  review: { flagged: readonly boolean[] };
}

/** A hunk of a file; `hunk` is -1 for a place before the file's first hunk (a change card). */
export interface HunkPlace {
  path: string;
  hunk: number;
}

/**
 * The flagged hunk after (`delta` 1) or before (-1) `from` over the files in order, without wrapping
 * around; with no `from` the first or the last one. Undefined when there is none.
 */
export function stepFlagged(
  files: readonly FlagFile[],
  from: HunkPlace | undefined,
  delta: 1 | -1,
): HunkPlace | undefined {
  const places = files.flatMap(({ path, review }) =>
    review.flagged.flatMap((flagged, hunk) => (flagged ? [{ path, hunk }] : [])),
  );
  if (delta === 1) return places.find((p) => from === undefined || after(files, p, from));
  return places.reverse().find((p) => from === undefined || after(files, from, p));
}

/** Whether `a` lies after `b` in the order of `files`; a place is not after itself. */
function after(files: readonly FlagFile[], a: HunkPlace, b: HunkPlace): boolean {
  const index = (p: HunkPlace) => files.findIndex((f) => f.path === p.path);
  const [ia, ib] = [index(a), index(b)];
  return ia !== ib ? ia > ib : a.hunk > b.hunk;
}
