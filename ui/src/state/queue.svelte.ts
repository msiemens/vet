// The changesets the repository offers, as the queue and the changeset switcher list them. The
// backend's list is shown as it is; what each row says is worked out where it is drawn. The local
// changesets come at once; the pull requests, which the platform has to be asked for, follow, and
// the lines each row changes come last, row by row, as the backend counts them.

import { SvelteMap } from "svelte/reactivity";
import type { ChangesetKey } from "../generated/ChangesetKey";
import type { ChangesetRow } from "../generated/ChangesetRow";
import type { LineDelta } from "../generated/LineDelta";
import type { PullRequestRow } from "../generated/PullRequestRow";
import type { PullRequestSource } from "../generated/PullRequestSource";
import type { PullRequestUpdate } from "../generated/PullRequestUpdate";
import { Debounced } from "../lib/debounced";
import { keyId } from "./changesetKey";
import { opened } from "./opened.svelte";
import { changesetLines, fetchRemotes, listChangesets, listPullRequests } from "./projectRequests";
import { type QueueActions, queueEvent, withDetails } from "./queueEvents";
import { queueEntries, queueRows } from "./queueEntries";
import { Latest } from "./resource";
import { session } from "./session.svelte";

/** A list shown follows a version or a mark this long after the last one. */
const FOLLOW_AFTER_MS = 100;

class QueueStore {
  /** The working tree and the branches. */
  local = $state.raw<ChangesetRow[]>([]);
  pullRequests = $state.raw<PullRequestRow[]>([]);
  /** How asking each GitHub repository of the project went; empty until the platform has answered. */
  sources = $state.raw<PullRequestSource[]>([]);
  /** A refresh is fetching the remotes. */
  refreshing = $state(false);
  /** The local changesets are being listed. */
  listingLocal = $state(false);
  /** The platform is being asked for the pull requests. */
  listingPullRequests = $state(false);
  /**
   * The lines the rows of the list change, by `keyId`, as far as the backend has counted them: `null` where
   * there are none to count. A row that is not in it is not counted yet.
   */
  lines = new SvelteMap<string, LineDelta | null>();
  /** Only the latest load of each kind counts. */
  private reads = { local: new Latest(), pullRequests: new Latest(), lines: new Latest() };
  /**
   * The platform is being asked for the pull requests. A request for another listing while it is on its
   * way marks it `again`, and one more follows when it is done: a burst of events costs at most two.
   */
  private listing?: { again: boolean; done: Promise<void> };
  private following = new Debounced();

  /**
   * The queue's rows: the local changesets and the pull requests, each with the lines and progress it
   * shows, the changeset on view's live. The queue, the switcher and Search Everywhere read this.
   */
  entries = $derived(
    queueEntries(
      queueRows(this.local, this.pullRequests),
      opened.session && keyId(opened.session.key),
      opened.session?.wholeFrame,
      this.lines,
    ),
  );

  /** The list follows the changeset on view for as long as a project is open. */
  private get projectOpen() {
    return session.repo !== undefined;
  }

  constructor() {
    session.onConnect((backend) => {
      const actions: QueueActions = {
        load: (recount) => void this.load(recount),
        loadPullRequests: () => void this.loadPullRequests(),
        takeLines: (changesets, lines) => this.takeLines(changesets, lines),
        takeDetails: (changed) => this.takeDetails(changed),
        follow: () => this.follow(),
      };
      backend.onEvent((event) => {
        if (event.project !== backend.project) return;
        // The branches are the local list's alone: the pull requests are listed when the queue is
        // shown and on a refresh, not on each fetch. What the details of a listed pull request add
        // to its row is patched in place, not listed again.
        queueEvent(event, actions);
      });
      // Another credential may list other pull requests, or list them where it could not before.
      backend.onAccountsChanged(() => {
        if (backend.project !== undefined) void this.loadPullRequests();
      });
    });
    session.onResync(() => {
      if (session.repo !== undefined) void this.loadAll();
    });
    session.onClose(() => {
      this.reads.local.cancel();
      this.reads.pullRequests.cancel();
      this.reads.lines.cancel();
      this.local = [];
      this.pullRequests = [];
      this.sources = [];
      this.listingLocal = false;
      this.listingPullRequests = false;
      this.listing = undefined;
      this.lines.clear();
      this.following.cancel();
    });
  }

  /** Lists the local changesets again once a burst of marks pauses, while a project is open. */
  private follow() {
    if (!this.projectOpen) return;
    this.following.schedule(FOLLOW_AFTER_MS, () => void this.load(false));
  }

  /** Something of the list is being fetched or listed. */
  get loading() {
    return this.refreshing || this.listingLocal || this.listingPullRequests;
  }

  /** Whether a read the list follows a burst of marks with is waiting to run. */
  get settling() {
    return this.following.pending;
  }

  /** Lists the local changesets. Rows already counted are counted again only with `recount`: a thread changing moves no branch. */
  async load(recount = true) {
    const isLatest = this.reads.local.begin();
    this.listingLocal = true;
    const listed = await listChangesets();
    if (!isLatest()) return;
    this.listingLocal = false;
    if (!listed) return;
    this.local = listed.rows;
    void this.countLines(recount);
  }

  /**
   * Asks the platform for the pull requests, which takes a moment: the ones listed stay until it answers.
   * Never more than one request is on its way; asking meanwhile has one more follow it, which a failure
   * does not.
   */
  loadPullRequests(): Promise<void> {
    if (this.listing) {
      this.listing.again = true;
      return this.listing.done;
    }
    const listing = { again: false, done: Promise.resolve() };
    this.listing = listing;
    listing.done = this.listPullRequests(listing).finally(() => {
      if (this.listing === listing) this.listing = undefined;
    });
    return listing.done;
  }

  private async listPullRequests(listing: { again: boolean }) {
    do {
      listing.again = false;
      const isLatest = this.reads.pullRequests.begin();
      this.listingPullRequests = true;
      const listed = await listPullRequests();
      if (!isLatest()) return;
      this.listingPullRequests = false;
      if (listed) {
        this.pullRequests = listed.rows;
        this.sources = listed.sources;
        void this.countLines(true);
      }
    } while (this.listing?.again);
  }

  /** The local changesets show as soon as they are listed, the pull requests when the platform answers. */
  async loadAll() {
    await Promise.all([this.load(), this.loadPullRequests()]);
  }

  /**
   * Shows the queue: lists the local changesets and reads the pull request list. Showing the queue is
   * not a reason to ask the platforms, so this reads the list that is; a refresh is asked for.
   */
  async show() {
    await this.loadAll();
  }

  /** Fetches the remotes, then lists again, the pull requests too. */
  async refresh() {
    if (this.refreshing) return;
    this.refreshing = true;
    const before = this.reads.local.mark();
    try {
      await fetchRemotes();
      // A fetch that brought something announced `changesetsChanged`, which listed the local ones already.
      await Promise.all([this.reads.local.movedSince(before) ? undefined : this.load(), this.loadPullRequests()]);
    } finally {
      this.refreshing = false;
    }
  }

  /**
   * Has the backend count the lines of every listed row, in one request, so a screen with many
   * branches costs one round trip. Every pull request too, so the queue and the review view use the
   * same base rule; the platform's own numbers are not shown. The working tree is not counted: its
   * manifest is live. A later listing starts over; the numbers shown stay until the new ones arrive.
   */
  private async countLines(recount: boolean) {
    const isLatest = this.reads.lines.begin();
    const wanted = [
      ...this.local.filter((row) => row.key.type !== "workingTree").map((row) => row.key),
      ...this.pullRequests.map((row) => row.key),
    ].filter((key) => recount || !this.lines.has(keyId(key)));
    if (!wanted.length) return;
    const lines = await changesetLines(wanted);
    if (!isLatest() || !lines) return;
    this.takeLines(wanted, lines);
  }

  /** Shows the counted lines of `changesets`, in order: a pushed chunk, or the request's answer. */
  private takeLines(changesets: readonly ChangesetKey[], lines: readonly (LineDelta | null | undefined)[]) {
    changesets.forEach((key, at) => this.lines.set(keyId(key), lines[at] ?? null));
  }

  /** Shows what the details that were read add to the rows they changed, leaving the rest. */
  private takeDetails(changed: readonly PullRequestUpdate[]) {
    this.pullRequests = withDetails(this.pullRequests, changed);
  }
}

export const queue = new QueueStore();
