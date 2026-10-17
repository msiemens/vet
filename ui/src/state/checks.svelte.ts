// The checks (CI runs) of the commits the review view shows, as the platform of the pull request
// on view has them. The backend refreshes them itself while the review screen shows the changeset
// and pushes each round (SPEC §7.5, A13); this only applies what it hears.

import { SvelteMap } from "svelte/reactivity";
import type { CommitChecks } from "../generated/CommitChecks";
import type { SessionEvent } from "./projectEvents";
import { notices } from "./notices.svelte";

class Checks {
  /** By full commit id; a commit the platform has no checks of has an entry without runs. */
  items = new SvelteMap<string, CommitChecks>();
  /** Unix seconds: the platform limits requests until then, so what is shown is older than it says. */
  limitedUntil = $state<number>();
  /** Unix seconds: when the checks were last answered for. */
  loadedAt = $state<number>();
  /** The reason last shown, so the same one does not come again with every round. */
  private lastError?: string;

  /** The checks of the commit with the full id `id`, if they are known. */
  of(id: string | undefined): CommitChecks | undefined {
    return id === undefined ? undefined : this.items.get(id);
  }

  hear(event: SessionEvent) {
    if (event.type === "checksChanged")
      this.apply(event.checks, event.error ?? undefined, event.limitedUntil ?? undefined);
  }

  /** The backend read the checks of the changeset on view again: what it answered for replaces. */
  private apply(checks: readonly CommitChecks[], error: string | undefined, limitedUntil: number | undefined) {
    this.items.clear();
    for (const commit of checks) this.items.set(commit.commit, commit);
    this.limitedUntil = limitedUntil;
    if (checks.length) this.loadedAt = Math.floor(Date.now() / 1000);
    // A limit is said by the Pipelines tool and the footer; the toast would only repeat it.
    if (error !== this.lastError && error && limitedUntil === undefined) {
      notices.show(`Could not load the checks: ${error}`);
    }
    this.lastError = error;
  }
}

export type SessionChecks = Checks;

export const newChecks = (): SessionChecks => new Checks();
