// The versions of a session's open review, as the backend lists them: the timeline's rows. Only the
// review screen shows them, so they are read while it is shown.

import type { VersionInfo } from "../generated/VersionInfo";
import type { Backend, SessionEvent } from "./backend";
import { notices } from "./notices.svelte";
import { Resource } from "./resource";

// A settled snapshot or a base move adds a version; a mark changes where the review last was.
const RELOAD_ON: readonly SessionEvent["type"][] = ["snapshotSettled", "reviewStarted", "reviewChanged"];

class Versions {
  /** Oldest first; none while browsing, as versions are recorded only while a review is open. */
  items = $state.raw<VersionInfo[]>([]);
  private following = false;
  private backend: () => Backend;
  private reads = new Resource<VersionInfo[]>({
    apply: (versions) => (this.items = versions),
    failed: (error) => notices.failed("Could not load the timeline", error),
  });

  constructor(backend: () => Backend) {
    this.backend = backend;
  }

  /**
   * Reads the versions and holds them. A command that needs them opens the review screen first, whose
   * start already reads them; this joins that read instead of asking again, and reads only when none
   * is underway.
   */
  async load(): Promise<VersionInfo[]> {
    const running = this.reads.running;
    if (running) await running;
    else await this.reads.load(() => this.backend().versions());
    return this.items;
  }

  follow() {
    this.following = true;
    void this.reads.load(() => this.backend().versions());
  }

  unfollow() {
    this.following = false;
    this.reads.cancel();
    this.items = [];
  }

  hear(event: SessionEvent) {
    if (this.following && RELOAD_ON.includes(event.type)) void this.reads.load(() => this.backend().versions());
  }

  resync() {
    if (this.following) void this.reads.load(() => this.backend().versions());
  }
}

export type SessionVersions = Versions;

export const newVersions = (backend: () => Backend): SessionVersions => new Versions(backend);
