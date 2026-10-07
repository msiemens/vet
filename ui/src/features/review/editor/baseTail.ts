// CodeMirror's chunks cover whole lines. Where a text ends without a final line break, the last line
// it adds is not followed by one, so the chunk reaches past the end of the base text. The backend
// shapes its changes so that this ends inside the line before it (`vet_app::editor`), except for an
// empty base text, which has no line before: there the chunk covers the base's one empty line, and
// a unified view draws it as a deleted row, one row more than the lines of the diff.

/** Whether `chunk` takes the empty last line of a base text of `baseLength` for deleted. */
export function replacesBaseTail(chunk: { fromA: number; toA: number }, baseLength: number): boolean {
  return chunk.fromA >= baseLength && chunk.toA > chunk.fromA;
}
