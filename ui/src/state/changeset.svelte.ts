// What the backend sent about the open changeset. Progress and review state are the
// backend's; this holds and distributes them (SPEC I5).

import { untrack } from "svelte";
import { SvelteMap } from "svelte/reactivity";
import type { BaseDiff } from "../generated/BaseDiff";
import type { FileChange } from "../generated/FileChange";
import type { ReviewUpdate } from "../generated/ReviewUpdate";
import type { Manifest } from "../generated/Manifest";
import type { RestoreFailure } from "../generated/RestoreFailure";
import type { ReviewFilter } from "../generated/ReviewFilter";
import type { NonTextKind } from "../generated/NonTextKind";
import type { BaseChoice } from "../generated/BaseChoice";
import type { ChangesetKey } from "../generated/ChangesetKey";
import type { CommentAt } from "../generated/CommentAt";
import type { View } from "../generated/View";
import type { CommitUnit } from "../generated/CommitUnit";
import type { Lens } from "../generated/Lens";
import { errorText } from "../lib/errorText";
import { pathsBelow } from "../lib/paths";
import { LatestLoad } from "./latestLoad";
import type { BaseCandidates } from "./backend";
import { commitToShow, neighbour } from "./commitLens";
import { keyId } from "./changesetKey";
import { currentFiles } from "./currentFiles";
import { isChangesetView, withChangesetBase } from "./manifestScope";
import { PendingOpen } from "./pendingOpen";
import { session } from "./session.svelte";
import { type Shown, isCurrent } from "./shown";
import { type Range, followed, rangeKey, rangeView } from "./timelineRange";
import { CHANGESET, commitView, interdiffView } from "./view";

type ReviewListener = (update: ReviewUpdate) => void;
/** Said when a request is refused or rejected because the views it came from are about to be replaced. */
const STALE_NOTICE = "Changes arrived — try again";
/** What happens to the changeset on view: another is asked for, announced to be fetched, or the ask came to nothing. */
export type Opening = { type: "requested" } | { type: "opened"; changeset: ChangesetKey } | { type: "unannounced" };

class ChangesetStore {
  /**
   * The manifest of the last refresh, with the changeset's completion kept current. Its
   * `files` are as of the refresh (paths and hunks, which only a new revision changes);
   * their review state is current in `file(path)`. Immutable snapshots, never deep proxies.
   */
  manifest = $state.raw<Manifest>();
  /** Dependency restores that failed: the analysis of those projects is limited. Pushed as they happen. */
  restoreFailures = $state.raw<RestoreFailure[]>([]);
  /** The current state of each file, one reactive cell per path: a review update invalidates only its file. */
  private cells = new SvelteMap<string, FileChange>();
  private listeners = new Set<ReviewListener>();

  /**
   * The versions the review view compares instead of showing the changeset; none for the changeset.
   * Set it with `setRange`.
   */
  range = $state.raw<Range>();
  /**
   * The commit the review view shows while the changeset is reviewed by commit: the index into
   * `commits`. A range takes its place, and the changeset's lens brings it back.
   */
  commitIndex = $state<number>();
  /** The commit the manifest is of; `commitIndex` moves first and this follows with the manifest. */
  shownCommit = $state<number>();
  /** The commit that was on view last, which by commit shows again. */
  private lastCommit = 0;
  /** The commits with their own progress, oldest first; loaded only while the changeset is reviewed by commit. */
  commits = $state.raw<CommitUnit[]>([]);
  private commitsSeq = 0;
  /** A manifest is being fetched. */
  private fetching = $state(false);
  private manifestLoads = new LatestLoad<{ manifest: Manifest; commit: number | undefined }>({
    busy: (on) => (this.fetching = on),
    apply: ({ manifest, commit }) => {
      this.cells.clear();
      for (const file of manifest.files) this.cells.set(file.path, file);
      this.manifest = manifest;
      this.shownCommit = commit;
      this.restoreFailures = manifest.restoreFailures;
      if (manifest.lens === "perCommit" && manifest.commits > 1) void this.loadCommits();
      else this.commits = [];
    },
  });
  /** Another changeset was asked for and is not announced yet. */
  private opening = $state(false);
  private pendingOpen = new PendingOpen((on) => (this.opening = on));
  private openingListeners = new Set<(opening: Opening) => void>();
  /** What requests about the files read: the interdiff of `range`, a commit, or the changeset. */
  get view(): View {
    return this.range || this.commitIndex === undefined ? rangeView(this.range) : commitView(this.commitIndex);
  }
  /** The range or commit the manifest is of, as a string; empty for the changeset. */
  viewKey = $derived(
    rangeKey(this.manifest?.interdiff) + (this.shownCommit === undefined ? "" : `c${this.shownCommit}`),
  );

  /**
   * While the review view shows only the files that had hunks seen against an old base when it was
   * asked for: their paths. Re-confirming does not shrink it, so the view stays as it is.
   */
  baseChanged = $state.raw<ReadonlySet<string>>();

  files = $derived(this.manifest?.files ?? []);
  /** Same array while only review state changes. */
  paths = $derived(this.files.map((f) => f.path));
  /** The files the Changes tab shows: all of them, or those of `baseChanged`. */
  shownFiles = $derived(this.baseChanged ? this.files.filter((f) => this.baseChanged?.has(f.path)) : this.files);

  /** A file as it is now, with its current review state and completion. */
  file(path: string): FileChange | undefined {
    return this.cells.get(path);
  }

  /** The manifest with every file's current state, for building views; reads nothing reactively. */
  currentManifest(): Manifest | undefined {
    return untrack(() => {
      const manifest = this.manifest;
      return manifest && { ...manifest, files: currentFiles(manifest.files, (p) => this.cells.get(p)) };
    });
  }

  /** The files with their current review state, for reading at the moment of use; reads nothing reactively. */
  currentFiles(): FileChange[] {
    return this.currentManifest()?.files ?? [];
  }

  constructor() {
    session.onConnect((backend) =>
      backend.onEvent((event) => {
        if (event.project !== backend.project) return;
        if (event.type === "changesetOpened") {
          if (!this.pendingOpen.announced(keyId(event.changeset))) return;
          this.clear();
          this.notifyOpening({ type: "opened", changeset: event.changeset });
          void this.refresh().catch((error: unknown) => (session.notice = errorText(error)));
        } else if (event.type === "snapshotSettled" || event.type === "targetChanged") void this.refresh();
        else if (event.type === "lensChanged") void this.followLens(event.lens);
        // A mark made in a range or a commit is announced as that of the changeset's file, which they do not show.
        else if (event.type === "reviewChanged" && event.revision === this.manifest?.revision) {
          if (isChangesetView(this.manifest, this.shownCommit)) this.applyReview(event.update);
          else this.applyTotals(event.update);
        } else if (event.type === "reviewStarted" && this.manifest)
          this.manifest = { ...this.manifest, review: event.review };
        else if (event.type === "restoreFailed") this.restoreFailures = event.failures;
        else if (event.type === "baseChanged" && this.manifest)
          this.manifest = withChangesetBase(this.manifest, event.base, this.shownCommit);
      }),
    );
    session.onResync(() => {
      if (this.manifest) this.refresh().catch((error: unknown) => (session.notice = errorText(error)));
    });
    session.onClose(() => this.clear());
  }

  /**
   * Drops everything of the changeset on view, for the views to show that the next one is loading.
   * A manifest fetch underway is of the changeset dropped and counts for nothing.
   */
  private clear() {
    this.manifestLoads.cancel();
    this.manifest = undefined;
    this.range = undefined;
    this.baseChanged = undefined;
    this.commitIndex = this.shownCommit = undefined;
    this.lastCommit = 0;
    this.commits = [];
    this.commitsSeq++;
    this.restoreFailures = [];
    this.cells.clear();
  }

  /**
   * The first manifest of a project or of the changeset just opened is being fetched: nothing of the
   * changeset is known, and the views show that instead of what was on view before.
   */
  get loading(): boolean {
    return this.opening || (this.fetching && !this.manifest);
  }

  /**
   * Called when a changeset is asked for (whatever belongs to the changeset on view is to be dropped
   * now), when the backend announced the one on view (to be read anew; not an earlier request's while a
   * newer is pending) and when it answered without announcing one (the changeset on view stays, to be
   * read again). Returns the unsubscribe function.
   */
  onOpening(listener: (opening: Opening) => void): () => void {
    this.openingListeners.add(listener);
    return () => this.openingListeners.delete(listener);
  }

  private notifyOpening(opening: Opening) {
    for (const listener of this.openingListeners) listener(opening);
  }

  /**
   * Fetches the manifest again; the revision may have moved. Only the latest fetch counts: an older
   * one that settles later gives the latest's manifest. Resolves with nothing when the project was
   * closed meanwhile.
   */
  async refresh(): Promise<Manifest | undefined> {
    const loaded = await this.manifestLoads.load(() => this.fetchManifest());
    return loaded?.manifest;
  }

  /**
   * The manifest of the view. A live range is moved to the latest version first, which only the
   * manifest says, and the commit the lens asks for is shown instead of the changeset, as the
   * manifest says too. A range or commit the backend refuses (its versions or commit are gone) gives
   * way to the changeset.
   */
  private async fetchManifest(): Promise<{ manifest: Manifest; commit: number | undefined }> {
    const backend = session.connected();
    for (let attempt = 0; ; attempt++) {
      const range = this.range;
      const commit = range ? undefined : this.commitIndex;
      let manifest: Manifest;
      try {
        manifest = await backend.manifest(this.view);
      } catch (error) {
        if (!range && commit === undefined) throw error;
        if (range) this.range = undefined;
        else this.commitIndex = undefined;
        session.notice = errorText(error);
        continue;
      }
      const latest = manifest.review.state === "open" ? manifest.review.version : undefined;
      if (range?.live && latest !== undefined && range.to !== latest && attempt < 3) {
        this.range = followed(range, latest);
        continue;
      }
      if (!range) {
        const wanted = commitToShow(manifest.lens, manifest.commits, this.lastCommit);
        if (wanted !== commit && attempt < 3) {
          this.commitIndex = wanted;
          continue;
        }
      }
      return { manifest, commit };
    }
  }

  /** Reads the commits again, with their progress. Only the latest reading counts. */
  async loadCommits() {
    const seq = ++this.commitsSeq;
    try {
      const commits = await session.connected().commits();
      if (seq === this.commitsSeq) this.commits = commits;
    } catch (error) {
      if (seq === this.commitsSeq) session.notice = `Could not load the commits: ${errorText(error)}`;
    }
  }

  /** Shows what the commit at `index` changed, and fetches its manifest. */
  async showCommit(index: number) {
    this.lastCommit = index;
    this.range = undefined;
    this.commitIndex = index;
    try {
      await this.refresh();
    } catch (error) {
      session.notice = errorText(error);
    }
  }

  /**
   * Goes to the commit `delta` away from the one on view, if there is one; answers whether it did.
   * It does not wrap around: the first and the last commit are the ends.
   */
  stepCommit(delta: number): boolean {
    if (this.shownCommit === undefined) return false;
    const to = neighbour(this.shownCommit, delta, this.manifest?.commits ?? 0);
    if (to === undefined) return false;
    void this.showCommit(to);
    return true;
  }

  /**
   * Reviews the changeset in `lens`, which the backend remembers for it. The backend pushes `lensChanged`, which
   * shows the view that goes with it.
   */
  async showLens(lens: Lens) {
    try {
      await session.connected().setLens(lens);
    } catch (error) {
      session.notice = errorText(error);
    }
  }

  /** The changeset is reviewed in `lens` now, as this window or another chose: shows the view that goes with it. */
  private async followLens(lens: Lens) {
    this.range = undefined;
    this.commitIndex = lens === "perCommit" ? this.lastCommit : undefined;
    try {
      await this.refresh();
    } catch (error) {
      session.notice = errorText(error);
    }
  }

  /**
   * Shows the interdiff of `range`, or the changeset (or its lens's commit) for none, and fetches its
   * manifest. Only the working tree has a live end: a committed changeset's latest version does not
   * follow anything.
   */
  async setRange(range: Range | undefined) {
    if (range) this.baseChanged = undefined;
    const follows = this.manifest?.target.changeset.type === "workingTree";
    this.range = range && { ...range, live: range.live && follows };
    if (range) this.commitIndex = undefined;
    try {
      await this.refresh();
    } catch (error) {
      session.notice = errorText(error);
    }
  }

  /** What the manifest on screen is of, which the views are built for; none before the first manifest. Reads nothing reactively. */
  currentShown(): Shown | undefined {
    return untrack(() => {
      const manifest = this.manifest;
      if (!manifest) return undefined;
      const view = manifest.interdiff
        ? interdiffView(manifest.interdiff.from, manifest.interdiff.to)
        : this.shownCommit === undefined
          ? CHANGESET
          : commitView(this.shownCommit);
      return { revision: manifest.revision, view };
    });
  }

  /**
   * Whether a request built for `shown` may go out: the manifest is still the one it was built from and
   * no other view has been asked for since. If not, the views it came from are about to be replaced
   * and the viewer is told so.
   */
  admit(shown: Shown): boolean {
    const manifest = this.manifest;
    const asked = manifest && { revision: manifest.revision, view: this.view };
    if (isCurrent(shown, asked)) return true;
    session.notice = STALE_NOTICE;
    return false;
  }

  /** Flips one hunk of the file in `shown`, the view the caller displays; the backend rejects a stale revision. */
  async toggleHunk(shown: Shown, path: string, hunk: number) {
    if (!this.admit(shown)) return;
    await this.review(shown, session.connected().toggleHunk(shown.revision, path, hunk, shown.view));
  }

  /** Flips the flag of one hunk. */
  async toggleFlag(shown: Shown, path: string, hunk: number) {
    if (!this.admit(shown)) return;
    await this.review(shown, session.connected().toggleFlag(shown.revision, path, hunk, shown.view));
  }

  /** Flips the acknowledgement of one non-text item. */
  async toggleItem(shown: Shown, path: string, kind: NonTextKind) {
    if (!this.admit(shown)) return;
    await this.review(shown, session.connected().toggleItem(shown.revision, path, kind, shown.view));
  }

  /** Marks everything of a file seen, or takes every mark back if it all is seen already. */
  async toggleFileSeen(shown: Shown, path: string) {
    if (!this.admit(shown)) return;
    await this.review(shown, session.connected().toggleFileSeen(shown.revision, path, shown.view));
  }

  /** Marks everything of every file of the view below the directory `dir` seen, or takes every mark back. */
  async setDirectorySeen(shown: Shown, dir: string, seen: boolean) {
    if (!this.admit(shown)) return;
    const paths = pathsBelow(this.paths, dir);
    if (paths.length)
      await this.review(shown, session.connected().setFilesSeen(shown.revision, paths, seen, shown.view));
  }

  /**
   * Marks the hunks of a file seen against an old base seen again. A range of versions has no such
   * request: its hunks are marked one by one, as the backend flips them.
   */
  async reconfirmFile(shown: Shown, path: string) {
    if (!this.admit(shown)) return;
    if (shown.view.type === "changeset") {
      await this.review(shown, session.connected().reconfirmFile(shown.revision, path));
      return;
    }
    const hunks = this.file(path)?.review.hunks ?? [];
    for (const [hunk, state] of hunks.entries()) {
      if (state === "seenAgainstOldBase") await this.toggleHunk(shown, path, hunk);
    }
  }

  /** The files that have hunks seen against an old base, as they are now. */
  baseChangedFiles(): string[] {
    return this.paths.filter((path) => this.file(path)?.review.hunks.includes("seenAgainstOldBase"));
  }

  /** Shows only the files with hunks seen against an old base in the review view, which then shows the changeset. */
  async showBaseChanged() {
    if (this.range) await this.setRange(undefined);
    this.baseChanged = new Set(this.baseChangedFiles());
  }

  /** Shows every file again. */
  showAll() {
    this.baseChanged = undefined;
  }

  /** Re-confirms every hunk seen against an old base of the changeset. */
  async reconfirmAll() {
    const shown = this.currentShown();
    if (!shown) return;
    for (const path of this.baseChangedFiles()) {
      // A request that failed refetched the manifest: the rest asks again.
      if (this.manifest?.revision !== shown.revision) return;
      await this.reconfirmFile(shown, path);
    }
  }

  /** The base diff of a hunk seen against an old base; `undefined` after showing why it could not be had. */
  async baseDiff(shown: Shown, path: string, hunk: number): Promise<BaseDiff | undefined> {
    if (!this.admit(shown)) return undefined;
    try {
      return await session.connected().baseDiff(shown.revision, path, hunk);
    } catch (error) {
      this.reportFailure(error, shown.revision);
      return undefined;
    }
  }

  /** Where a comment on a part of the file attaches in `shown`; rejects if `shown` is not current. */
  async commentTarget(shown: Shown, path: string, at: CommentAt) {
    if (!this.admit(shown)) throw new Error(STALE_NOTICE);
    return session.connected().commentTarget(shown.revision, path, at, shown.view);
  }

  /** The changesets on offer and the one on view; `undefined` after showing why they could not be listed. */
  async listChangesets() {
    try {
      return await session.connected().listChangesets();
    } catch (error) {
      session.notice = errorText(error);
      return undefined;
    }
  }

  /** The open pull requests on offer; `undefined` after showing why they could not be listed. */
  async listPullRequests() {
    try {
      return await session.connected().listPullRequests();
    } catch (error) {
      session.notice = errorText(error);
      return undefined;
    }
  }

  /** The lines a row of the list changes; `undefined` where there are none to count or counting failed. */
  async changesetLines(key: ChangesetKey) {
    try {
      return await session.connected().changesetLines(key);
    } catch {
      return undefined;
    }
  }

  /**
   * Puts a changeset on view. What is on view belongs to another changeset from now on and is dropped; the
   * backend pushes `changesetOpened`, which fetches the manifest. If it answers without (the changeset
   * is on view already, or it refused), the changeset on view is read again.
   */
  async openChangeset(key: ChangesetKey) {
    const backend = session.connected();
    const token = this.pendingOpen.begin(keyId(key));
    this.clear();
    this.notifyOpening({ type: "requested" });
    try {
      await backend.openChangeset(key);
    } catch (error) {
      session.notice = errorText(error);
    }
    if (!this.pendingOpen.settle(token)) return;
    this.notifyOpening({ type: "unannounced" });
    await this.refresh().catch((error: unknown) => (session.notice = errorText(error)));
  }

  /**
   * Compares against `choice`. The backend remaps the review state and pushes
   * `snapshotSettled`, which fetches the new manifest; a choice that does not resolve shows the
   * backend's message.
   */
  async setBase(choice: BaseChoice) {
    try {
      await session.connected().setBase(choice);
    } catch (error) {
      session.notice = errorText(error);
    }
  }

  /** Fetches the remotes. The footer shows the task; the base's `behind` follows with `baseChanged`. */
  async fetch() {
    try {
      await session.connected().fetch();
    } catch (error) {
      session.notice = errorText(error);
    }
  }

  /**
   * Ends the open review and says how it ended. The backend pushes `snapshotSettled`, which
   * fetches the manifest again, back to browsing.
   */
  async finishReview() {
    try {
      this.range = undefined;
      const complete = await session.connected().finishReview();
      session.notice = complete ? "Review finished: everything was seen" : "Review finished with unseen changes left";
    } catch (error) {
      session.notice = errorText(error);
    }
  }

  /**
   * Replaces the repository's review filter. The backend pushes `snapshotSettled`, which fetches the new
   * manifest. Resolves with the backend's message if it refused the filter (a bad glob).
   */
  async setFilter(filter: ReviewFilter): Promise<string | undefined> {
    try {
      await session.connected().setProjectFilter(filter);
      return undefined;
    } catch (error) {
      return errorText(error);
    }
  }

  /** Excludes `path` (a file, or a folder with `dir`) from the review. As with `setFilter`, the backend then pushes `snapshotSettled`. */
  async excludePath(path: string, dir: boolean) {
    try {
      await session.connected().excludePath(path, dir);
    } catch (error) {
      session.notice = errorText(error);
    }
  }

  /** Limits the review to the folder `path`. */
  async reviewOnly(path: string) {
    const filter = this.manifest?.filter;
    if (!filter) return;
    const error = await this.setFilter({ ...filter, roots: [path] });
    if (error) session.notice = error;
  }

  /** The children of a directory of the target snapshot; `undefined` after showing why it failed. */
  async listDirectory(path: string) {
    try {
      return await session.connected().listDirectory(path);
    } catch (error) {
      session.notice = errorText(error);
      return undefined;
    }
  }

  /** What a base can be picked from; `undefined` after showing why it failed. */
  async baseCandidates(): Promise<BaseCandidates | undefined> {
    try {
      return await session.connected().baseCandidates();
    } catch (error) {
      session.notice = errorText(error);
      return undefined;
    }
  }

  /**
   * Applies the review state a request returned. A rejected request is normally a
   * stale revision: refetch the manifest and tell the viewer to repeat the action.
   */
  private async review(sent: Shown, request: Promise<ReviewUpdate | ReviewUpdate[]>) {
    try {
      for (const update of [await request].flat()) this.applyReview(update);
    } catch (error) {
      await this.refresh().catch(() => undefined);
      this.reportFailure(error, sent.revision);
    }
  }

  /**
   * Tells the viewer why a request failed. When the manifest has moved on from the revision the
   * request was sent for, the views are about to be replaced and the action has to be repeated.
   */
  reportFailure(error: unknown, sent?: number) {
    const current = this.manifest?.revision;
    session.notice =
      sent !== undefined && current !== undefined && current !== sent
        ? STALE_NOTICE
        : `Request failed: ${errorText(error)}`;
  }

  applyReview(update: ReviewUpdate) {
    const manifest = this.manifest;
    if (!manifest) return;
    const file = this.cells.get(update.path);
    if (file) this.cells.set(update.path, { ...file, review: update.review, completion: update.completion });
    this.manifest = {
      ...manifest,
      completion: update.changeset,
      // Only the changeset's own view says how far the changeset as a whole is.
      changesetCompletion: isChangesetView(manifest, this.shownCommit)
        ? update.changeset
        : manifest.changesetCompletion,
    };
    for (const listener of this.listeners) listener(update);
    if (manifest.lens === "perCommit" && manifest.commits > 1) void this.loadCommits();
  }

  /** A mark in the changeset's file, which a range or commit on screen does not show: the changeset's progress and the commits' follow. */
  private applyTotals(update: ReviewUpdate) {
    if (!this.manifest) return;
    this.manifest = { ...this.manifest, changesetCompletion: update.changeset };
    void this.loadCommits();
  }

  /** Called after each review update, whether it came from a request or an event. */
  onReview(listener: ReviewListener): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }
}

export type Changeset = ChangesetStore;

export const changeset = new ChangesetStore();
