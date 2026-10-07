// What the views of one manifest revision ask the backend for about files. A reply of another
// revision is dropped: the owner builds new views when the manifest moves.

import type { BlobContent } from "../../../generated/BlobContent";
import type { FileChange } from "../../../generated/FileChange";
import type { NonTextKind } from "../../../generated/NonTextKind";
import type { Side } from "../../../generated/Side";
import type { TextContent } from "../../../generated/TextContent";
import { NO_HIGHLIGHTS, decodeHighlights } from "../../../lib/highlights.ts";
import type { Backend } from "../../../state/backend";
import { atRevision } from "../../../state/revision.ts";
import type { Loaded } from "./mount";

/** The reply belonged to another revision than the view that asked. */
export class StaleRevision extends Error {
  constructor() {
    super("stale revision");
  }
}

export interface FileSource {
  /** Both sides of a changed file with their highlights. */
  contents(file: FileChange): Promise<Loaded>;
  /** The file's text in the target snapshot, for the read-only view. */
  text(path: string): Promise<TextContent>;
  /** An image or other blob of a non-text item. */
  itemBlob(path: string, kind: NonTextKind, side: Side): Promise<BlobContent>;
  /** The target line that shows `line` of `side`'s text. */
  targetLine(side: Side, path: string, line: number): Promise<number>;
}

export function fileSource(backend: Backend, revision: number): FileSource {
  return {
    async contents(file) {
      // A file without text lines (binary, content unavailable) has nothing to fetch; the backend refuses it.
      if (!file.hunks.length && !file.baseLines && !file.targetLines)
        return {
          base: "",
          target: "",
          hunks: [],
          changes: [],
          baseHighlights: NO_HIGHLIGHTS,
          targetHighlights: NO_HIGHLIGHTS,
        };
      const reply = atRevision(revision, await backend.fileContents(file.path));
      if (!reply) throw new StaleRevision();
      return {
        base: reply.base ?? "",
        target: reply.target ?? "",
        hunks: file.hunks,
        changes: reply.changes,
        baseHighlights: decodeHighlights(reply.baseHighlights),
        targetHighlights: decodeHighlights(reply.targetHighlights),
      };
    },
    async text(path) {
      const reply = atRevision(revision, await backend.fileText(path));
      if (!reply) throw new StaleRevision();
      return reply.content;
    },
    itemBlob: (path, kind, side) => backend.itemBlob(revision, path, kind, side),
    targetLine: (side, path, line) => backend.targetLine(revision, side, path, line),
  };
}
