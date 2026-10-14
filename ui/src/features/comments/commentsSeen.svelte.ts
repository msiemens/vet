// When the viewer last had the Comments tool window of each pull request open, kept as a
// preference so it outlasts a restart.

import { Preference } from "../../lib/preferences.svelte";
import { type SeenTimes, parseSeen, withSeen } from "./seenTimes";

const kept = new Preference<SeenTimes>("app", "commentsSeen", parseSeen, {});

class CommentsSeen {
  /** Unix seconds, under a `seenKey`; absent while the tool window was never open there. */
  at(key: string): number | undefined {
    return kept.value[key];
  }

  mark(key: string, at: number) {
    if (this.at(key) === at) return;
    kept.value = withSeen(kept.value, key, at);
  }
}

export const commentsSeen = new CommentsSeen();
