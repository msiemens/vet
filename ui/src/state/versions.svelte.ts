// The versions of the open review, as the backend lists them: the timeline's rows.

import type { VersionInfo } from "../generated/VersionInfo";
import type { Backend } from "./backend";
import { followProject } from "./followProject";
import { whileReview } from "./reviewFollow";
import { notices } from "./notices.svelte";
import { Resource } from "./resource";
import { session } from "./session.svelte";

class VersionStore {
  /** Oldest first; none while browsing, as versions are recorded only while a review is open. */
  items = $state.raw<VersionInfo[]>([]);
  private reads = new Resource<VersionInfo[]>({
    apply: (versions) => (this.items = versions),
    failed: (error) => notices.failed("Could not load the timeline", error),
  });

  constructor() {
    whileReview((backend) => this.start(backend));
  }

  /**
   * Reads the versions and holds them. A command that needs them opens the review screen first, whose
   * start already reads them; this joins that read instead of asking again, and reads only when none
   * is underway.
   */
  async load(): Promise<VersionInfo[]> {
    const running = this.reads.running;
    if (running) await running;
    else {
      const backend = session.backend;
      if (backend) await this.reads.load(() => backend.versions());
    }
    return this.items;
  }

  /** Loads the versions and keeps them current; the returned function stops that. */
  private start(backend: Backend): () => void {
    const stop = followProject(backend, {
      // A settled snapshot or a base move adds a version; a mark changes where the review last was;
      // another changeset has versions of its own.
      reloadOn: ["snapshotSettled", "reviewStarted", "reviewChanged"],
      reload: () => void this.reads.load(() => backend.versions()),
      cancel: () => this.reads.cancel(),
      clear: () => (this.items = []),
    });
    return () => {
      stop();
      this.reads.cancel();
      this.items = [];
    };
  }
}

export const versions = new VersionStore();
