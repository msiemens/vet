// The versions of the open review, as the backend lists them: the timeline's rows.

import type { VersionInfo } from "../generated/VersionInfo";
import { errorText } from "../lib/errorText";
import type { Backend } from "./backend";
import { session } from "./session.svelte";

class VersionStore {
  /** Oldest first; none while browsing, as versions are recorded only while a review is open. */
  items = $state.raw<VersionInfo[]>([]);
  /** Only the latest reload counts. */
  private seq = 0;

  /** Loads the versions and keeps them current; the returned function stops that. */
  start(backend: Backend): () => void {
    void this.reload(backend);
    const off = backend.onEvent((event) => {
      if (event.project !== backend.project) return;
      // A settled snapshot or a base move adds a version; a mark changes where the review last was;
      // another changeset has versions of its own.
      if (
        event.type === "snapshotSettled" ||
        event.type === "reviewStarted" ||
        event.type === "reviewChanged" ||
        event.type === "changesetOpened"
      )
        void this.reload(backend);
    });
    return () => {
      off();
      this.seq++;
      this.items = [];
    };
  }

  private async reload(backend: Backend) {
    const seq = ++this.seq;
    try {
      const versions = await backend.versions();
      if (seq === this.seq) this.items = versions;
    } catch (error) {
      if (seq === this.seq) session.notice = `Could not load the timeline: ${errorText(error)}`;
    }
  }
}

export const versions = new VersionStore();
