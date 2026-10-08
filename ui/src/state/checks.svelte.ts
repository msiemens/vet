// The checks (CI runs) of the commits the review view shows, as the platform of the pull request on
// view has them. Commits whose checks are done are asked for once; while any runs they are asked
// for again every so often, and when a fetch or a push may have changed them.

import { SvelteMap } from "svelte/reactivity";
import type { CommitChecks } from "../generated/CommitChecks";
import { errorText } from "../lib/errorText";
import { session } from "./session.svelte";

/** How long a commit with a running check waits before it is asked about again. */
export const REFRESH_MS = 30_000;

class CheckStore {
  /** By full commit id; a commit the platform has no checks of has an entry without runs. */
  items = new SvelteMap<string, CommitChecks>();
  private wanted: readonly string[] = [];
  /** Only the latest load counts. */
  private seq = 0;
  private timer?: ReturnType<typeof setTimeout>;
  /** The reason last shown, so the same one does not come again with every load. */
  private lastError?: string;

  constructor() {
    session.onConnect((backend) =>
      backend.onEvent((event) => {
        if (event.project !== backend.project) return;
        if (event.type === "baseChanged" || event.type === "snapshotSettled") void this.load();
      }),
    );
    session.onResync(() => void this.load());
    session.onClose(() => {
      this.want([]);
      this.items.clear();
    });
  }

  /** The checks of the commit with the full id `id`, if they are known. */
  of(id: string | undefined): CommitChecks | undefined {
    return id === undefined ? undefined : this.items.get(id);
  }

  /** Asks for the checks of these commits (full ids), and keeps them current while any runs. */
  want(commits: readonly string[]) {
    this.wanted = commits;
    void this.load();
  }

  private async load() {
    clearTimeout(this.timer);
    const seq = ++this.seq;
    const open = this.wanted.filter((id) => {
      const state = this.items.get(id)?.state;
      return state === undefined || state === "pending";
    });
    if (!open.length || !session.backend) return;
    try {
      const { checks, error } = await session.backend.checks(open);
      if (seq !== this.seq) return;
      for (const commit of checks) this.items.set(commit.commit, commit);
      if (error !== this.lastError && error) session.notice = `Could not load the checks: ${error}`;
      this.lastError = error;
    } catch (error) {
      if (seq !== this.seq) return;
      const text = errorText(error);
      if (text !== this.lastError) session.notice = `Could not load the checks: ${text}`;
      this.lastError = text;
    }
    if (this.wanted.some((id) => this.items.get(id)?.state === "pending")) {
      this.timer = setTimeout(() => void this.load(), REFRESH_MS);
    }
  }
}

export const checks = new CheckStore();
