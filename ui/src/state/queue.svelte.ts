// The changesets the repository offers, as the queue and the changeset switcher list them. The
// backend's list is shown as it is; what each row says is worked out where it is drawn.

import type { ChangesetKey } from "../generated/ChangesetKey";
import type { ChangesetRow } from "../generated/ChangesetRow";
import { changeset } from "./changeset.svelte";
import { session } from "./session.svelte";

class QueueStore {
  rows = $state.raw<ChangesetRow[]>([]);
  /** The changeset on view. */
  onView = $state.raw<ChangesetKey>();
  /** A refresh is fetching the remotes. */
  refreshing = $state(false);
  /** Only the latest load counts. */
  private seq = 0;

  constructor() {
    session.onConnect((backend) =>
      backend.onEvent((event) => {
        if (event.project !== backend.project) return;
        // Which changeset is on view, what has a review, and how many threads are open.
        if (event.type === "changesetOpened" || event.type === "reviewStarted" || event.type === "threadsChanged")
          void this.load();
      }),
    );
    session.onClose(() => {
      this.seq++;
      this.rows = [];
      this.onView = undefined;
    });
  }

  async load() {
    const seq = ++this.seq;
    const listed = await changeset.listChangesets();
    if (!listed || seq !== this.seq) return;
    this.rows = listed.rows;
    this.onView = listed.onView;
  }

  /** Fetches the remotes, then lists again. */
  async refresh() {
    if (this.refreshing) return;
    this.refreshing = true;
    try {
      await changeset.fetch();
      await this.load();
    } finally {
      this.refreshing = false;
    }
  }
}

export const queue = new QueueStore();
