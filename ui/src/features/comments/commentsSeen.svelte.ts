// When the viewer last had the Comments tool window of each pull request open, kept in the
// browser storage so it outlasts a restart.

import { readStored, writeStored } from "../../lib/storage";

function stored(key: string): number | undefined {
  const at = Number(readStored(key));
  return Number.isFinite(at) && at > 0 ? at : undefined;
}

class CommentsSeen {
  /** What was marked in this window, which the storage also has. */
  private marked = $state<Record<string, number>>({});

  /** Unix seconds, under a `seenKey`; absent while the tool window was never open there. */
  at(key: string): number | undefined {
    return this.marked[key] ?? stored(key);
  }

  mark(key: string, at: number) {
    if (this.at(key) === at) return;
    writeStored(key, String(at));
    this.marked[key] = at;
  }
}

export const commentsSeen = new CommentsSeen();
