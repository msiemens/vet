// The checks (CI runs) of the commits the review view shows, as the platform of the pull request on
// view has them. Commits whose checks are done are asked for once; while any runs they are asked
// for again every so often, and when a fetch or a push may have changed them.

import { SvelteMap } from "svelte/reactivity";
import type { CommitChecks } from "../generated/CommitChecks";
import { Debounced } from "../lib/debounced";
import { errorText } from "../lib/errorText";
import { refreshDelay } from "./checkRefresh";
import { session } from "./session.svelte";

/** How long after a run was changed it is asked about once more: the platform has not shown it at once. */
const SETTLE_MS = 3_000;

class CheckStore {
  /** By full commit id; a commit the platform has no checks of has an entry without runs. */
  items = new SvelteMap<string, CommitChecks>();
  /** Unix seconds: the platform limits requests until then, so what is shown is older than it says. */
  limitedUntil = $state<number>();
  /** Unix seconds: when the checks were last answered for. */
  loadedAt = $state<number>();
  private wanted: readonly string[] = [];
  /** Only the latest load counts. */
  private seq = 0;
  private timer = new Debounced();
  private again = new Debounced();
  /** The reason last shown, so the same one does not come again with every load. */
  private lastError?: string;

  constructor() {
    session.onConnect((backend) =>
      backend.onEvent((event) => {
        if (event.project !== backend.project) return;
        if (event.type === "changesetsChanged" || event.type === "snapshotSettled") void this.load();
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

  /**
   * Asks again for these commits, whatever they said before: a run was changed on the platform,
   * which takes a moment to show it, so they are asked once more shortly after.
   */
  refresh(commits: readonly string[]) {
    void this.load(commits);
    this.again.schedule(SETTLE_MS, () => void this.load(commits));
  }

  private async load(again: readonly string[] = []) {
    this.timer.cancel();
    const seq = ++this.seq;
    const open = this.wanted.filter((id) => {
      const state = this.items.get(id)?.state;
      return state === undefined || state === "pending" || again.includes(id);
    });
    if (!open.length || !session.backend) return;
    try {
      const { checks, error, limitedUntil } = await session.backend.checks(open);
      if (seq !== this.seq) return;
      for (const commit of checks) this.items.set(commit.commit, commit);
      this.limitedUntil = limitedUntil;
      if (checks.length) this.loadedAt = Math.floor(Date.now() / 1000);
      // A limit is said by the Pipelines tool and the footer; the toast would only repeat it.
      if (error !== this.lastError && error && limitedUntil === undefined) {
        session.notice = `Could not load the checks: ${error}`;
      }
      this.lastError = error;
    } catch (error) {
      if (seq !== this.seq) return;
      const text = errorText(error);
      if (text !== this.lastError) session.notice = `Could not load the checks: ${text}`;
      this.lastError = text;
    }
    if (this.wanted.some((id) => this.items.get(id)?.state === "pending")) {
      this.timer.schedule(refreshDelay(this.limitedUntil, Date.now()), () => void this.load());
    }
  }
}

export const checks = new CheckStore();
