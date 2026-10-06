import type { FileChange } from "../../../generated/FileChange";

/** Lines the text hunks of `files` add and remove: one file's `+N −M`, or the whole changeset's. */
export function lineDelta(files: readonly FileChange[]): { added: number; removed: number } {
  let added = 0;
  let removed = 0;
  for (const file of files) {
    for (const h of file.hunks) {
      added += h.targetLen;
      removed += h.baseLen;
    }
  }
  return { added, removed };
}
