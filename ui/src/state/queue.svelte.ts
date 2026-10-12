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
import { changeset } from "./changeset.svelte";
import { keyId } from "./changesetKey";
import { session } from "./session.svelte";

/** How many rows have their lines counted at once. */
const LINE_COUNTS_AT_ONCE = 3;
/** A list shown follows a version or a mark this long after the last one. */
const FOLLOW_AFTER_MS = 300;

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
  private seq = { local: 0, pullRequests: 0, lines: 0 };
  /** How many times an account changed: a listing of the pull requests that began before the last change lists again. */
  private accountChanges = 0;
  /** How many views of the list are shown. */
  private watchers = $state(0);
  private following?: ReturnType<typeof setTimeout>;

  constructor() {
    session.onConnect((backend) => {
      backend.onEvent((event) => {
        if (event.project !== backend.project) return;
        // Which changeset is on view and what has a review. Where the branches are is the local list's
        // alone: the pull requests are listed when the queue is shown and on a refresh, not on each fetch.
        if (event.type === "changesetOpened" || event.type === "reviewStarted") void this.loadAll();
        else if (event.type === "changesetsChanged") void this.load();
        else if (event.type === "threadsChanged") void this.load(false);
        // A new version or a mark changes the version, the last reviewed version and the progress of the row on view.
        else if (event.type === "snapshotSettled" || event.type === "reviewChanged") this.follow();
      });
      // Another credential may list other pull requests, or list them where it could not before.
      backend.onAccountsChanged(() => {
        this.accountChanges++;
        if (backend.project !== undefined && !this.listingPullRequests) void this.loadPullRequests();
      });
    });
    session.onResync(() => {
      if (session.repo !== undefined) void this.loadAll();
    });
    session.onClose(() => {
      this.seq.local++;
      this.seq.pullRequests++;
      this.seq.lines++;
      this.local = [];
      this.pullRequests = [];
      this.sources = [];
      this.onView = undefined;
      this.listingLocal = false;
      this.listingPullRequests = false;
      this.lines.clear();
      clearTimeout(this.following);
    });
  }

  /**
   * The list is shown (the queue, or the switcher opened): it follows the events that change what it says
   * about the changeset on view. Returns the function that ends that.
   */
  watch(): () => void {
    this.watchers++;
    return () => this.watchers--;
  }

  /** Lists the local changesets again once a burst of marks pauses, if the list is shown. */
  private follow() {
    if (this.watchers === 0) return;
    clearTimeout(this.following);
    this.following = setTimeout(() => void this.load(false), FOLLOW_AFTER_MS);
  }

  /** Something of the list is being fetched or listed. */
  get loading() {
    return this.refreshing || this.listingLocal || this.listingPullRequests;
  }

  /** Lists the local changesets. Rows already counted are counted again only with `recount`: a thread changing moves no branch. */
  async load(recount = true) {
    const seq = ++this.seq.local;
    this.listingLocal = true;
    const listed = await changeset.listChangesets();
    if (seq !== this.seq.local) return;
    this.listingLocal = false;
    if (!listed) return;
    this.local = listed.rows;
    this.onView = listed.onView;
    void this.countLines(recount);
  }

  /** Asks the platform for the pull requests, which takes a moment: the ones listed stay until it answers. */
  async loadPullRequests() {
    const seq = ++this.seq.pullRequests;
    const changes = this.accountChanges;
    this.listingPullRequests = true;
    const listed = await changeset.listPullRequests();
    if (seq !== this.seq.pullRequests) return;
    this.listingPullRequests = false;
    if (listed) {
      this.pullRequests = listed.rows;
      this.sources = listed.sources;
      void this.countLines(true);
    }
    if (this.accountChanges !== changes) void this.loadPullRequests();
  }

  /** The local changesets show as soon as they are listed, the pull requests when the platform answers. */
  async loadAll() {
    await Promise.all([this.load(), this.loadPullRequests()]);
  }

  /** Fetches the remotes, then lists again, the pull requests too. */
  async refresh() {
    if (this.refreshing) return;
    this.refreshing = true;
    const before = this.seq.local;
    try {
      await changeset.fetch();
      // A fetch that brought something announced `changesetsChanged`, which listed the local ones already.
      await Promise.all([this.seq.local === before ? this.load() : undefined, this.loadPullRequests()]);
    } finally {
      this.refreshing = false;
    }
  }

  /**
   * Has the backend count the lines of every listed row that has none from the platform, a few at a time
   * and top of the list first. The working tree is not counted: its manifest is live. A later listing
   * starts over; the numbers shown stay until the new ones arrive.
   */
  private async countLines(recount: boolean) {
    const seq = ++this.seq.lines;
    const wanted = [
      ...this.local.filter((row) => row.key.type !== "workingTree").map((row) => row.key),
      ...this.pullRequests.filter((row) => row.lines === undefined).map((row) => row.key),
    ].filter((key) => recount || !this.lines.has(keyId(key)));
    let next = 0;
    const count = async () => {
      while (seq === this.seq.lines && next < wanted.length) {
        const key = wanted[next++];
        const lines = await changeset.changesetLines(key);
        if (seq !== this.seq.lines) return;
        this.lines.set(keyId(key), lines ?? null);
      }
    };
    await Promise.all(Array.from({ length: LINE_COUNTS_AT_ONCE }, count));
  }
}

export const queue = new QueueStore();
