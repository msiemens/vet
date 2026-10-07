// The changesets the repository offers, as the queue and the changeset switcher list them. The
// backend's list is shown as it is; what each row says is worked out where it is drawn. The local
// changesets come at once; the pull requests, which the platform has to be asked for, follow.

import type { ChangesetKey } from "../generated/ChangesetKey";
import type { ChangesetRow } from "../generated/ChangesetRow";
import type { PullRequestRow } from "../generated/PullRequestRow";
import type { PullRequestSource } from "../generated/PullRequestSource";
import { changeset } from "./changeset.svelte";
import { session } from "./session.svelte";

class QueueStore {
  /** The working tree and the branches. */
  local = $state.raw<ChangesetRow[]>([]);
  pullRequests = $state.raw<PullRequestRow[]>([]);
  /** How asking each GitHub repository of the project went; empty until the platform has answered. */
  sources = $state.raw<PullRequestSource[]>([]);
  /** The changeset on view. */
  onView = $state.raw<ChangesetKey>();
  /** A refresh is fetching the remotes. */
  refreshing = $state(false);
  /** Only the latest load of each kind counts. */
  private seq = { local: 0, pullRequests: 0 };

  constructor() {
    session.onConnect((backend) =>
      backend.onEvent((event) => {
        if (event.project !== backend.project) return;
        // Which changeset is on view, what has a review, and how many threads are open.
        if (event.type === "changesetOpened" || event.type === "reviewStarted") void this.loadAll();
        else if (event.type === "threadsChanged") void this.load();
      }),
    );
    session.onClose(() => {
      this.seq.local++;
      this.seq.pullRequests++;
      this.local = [];
      this.pullRequests = [];
      this.sources = [];
      this.onView = undefined;
    });
  }

  /** Lists the local changesets. */
  async load() {
    const seq = ++this.seq.local;
    const listed = await changeset.listChangesets();
    if (!listed || seq !== this.seq.local) return;
    this.local = listed.rows;
    this.onView = listed.onView;
  }

  /** Asks the platform for the pull requests, which takes a moment: the ones listed stay until it answers. */
  async loadPullRequests() {
    const seq = ++this.seq.pullRequests;
    const listed = await changeset.listPullRequests();
    if (!listed || seq !== this.seq.pullRequests) return;
    this.pullRequests = listed.rows;
    this.sources = listed.sources;
  }

  /** The local changesets show as soon as they are listed, the pull requests when the platform answers. */
  async loadAll() {
    await Promise.all([this.load(), this.loadPullRequests()]);
  }

  /** Fetches the remotes, then lists again. */
  async refresh() {
    if (this.refreshing) return;
    this.refreshing = true;
    try {
      await changeset.fetch();
      await this.loadAll();
    } finally {
      this.refreshing = false;
    }
  }
}

export const queue = new QueueStore();
