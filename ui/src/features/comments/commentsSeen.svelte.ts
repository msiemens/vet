// When the viewer last had the Comments tool window of each pull request open, kept as a
// preference so it outlasts a restart.

import { numberPreference } from "../../lib/preferences.svelte";

class CommentsSeen {
  /** Unix seconds, under a `seenKey`; absent while the tool window was never open there. */
  at(key: string): number | undefined {
    const at = numberPreference("app", key).value;
    return at !== undefined && at > 0 ? at : undefined;
  }

  mark(key: string, at: number) {
    if (this.at(key) === at) return;
    numberPreference("app", key).value = at;
  }
}

export const commentsSeen = new CommentsSeen();
