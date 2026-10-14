// The versions of the open review, as the backend lists them: the timeline's rows.

import type { VersionInfo } from "../generated/VersionInfo";
import type { Backend } from "./backend";
import { followProject, whileOpen } from "./followProject";
import { notices } from "./notices.svelte";
import { Resource } from "./resource";

class VersionStore {
  /** Oldest first; none while browsing, as versions are recorded only while a review is open. */
  items = $state.raw<VersionInfo[]>([]);
  private reads = new Resource<VersionInfo[]>({
    apply: (versions) => (this.items = versions),
    failed: (error) => notices.failed("Could not load the timeline", error),
  });

  constructor() {
    whileOpen((backend) => this.start(backend));
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
