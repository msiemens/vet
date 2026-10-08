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
import type { View } from "../generated/View";
import type { CommitUnit } from "../generated/CommitUnit";
import type { Lens } from "../generated/Lens";
import { errorText } from "../lib/errorText";
import type { BaseCandidates } from "./backend";
import { commitToShow, neighbour } from "./commitLens";
import { session } from "./session.svelte";
import { type Range, followed, rangeKey, rangeView } from "./timelineRange";
import { commitView } from "./view";

type ReviewListener = (update: ReviewUpdate) => void;

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
      return manifest && { ...manifest, files: manifest.files.map((f) => this.cells.get(f.path) ?? f) };
    });
  }

  constructor() {
    session.onConnect((backend) =>
      backend.onEvent((event) => {
        if (event.project !== backend.project) return;
        if (event.type === "changesetOpened") {
          // Versions, commits and what was seen against an old base are the changeset's own.
          this.range = undefined;
          this.baseChanged = undefined;
          this.commitIndex = undefined;
          this.lastCommit = 0;
          void this.refresh();
        } else if (event.type === "snapshotSettled" || event.type === "targetChanged") void this.refresh();
        else if (event.type === "lensChanged") void this.followLens(event.lens);
        // A mark made in a range or a commit is announced as that of the changeset's file, which they do not show.
        else if (event.type === "reviewChanged" && event.revision === this.manifest?.revision) {
          if (this.shownCommit !== undefined) this.applyTotals(event.update);
          else if (!this.range) this.applyReview(event.update);
        } else if (event.type === "reviewStarted" && this.manifest)
          this.manifest = { ...this.manifest, review: event.review };
        else if (event.type === "restoreFailed") this.restoreFailures = event.failures;
        else if (event.type === "baseChanged" && this.manifest) this.manifest = { ...this.manifest, base: event.base };
      }),
    );
    session.onResync(() => {
      if (this.manifest) this.refresh().catch((error: unknown) => (session.notice = errorText(error)));
    });
    session.onClose(() => {
      this.manifest = undefined;
      this.range = undefined;
      this.baseChanged = undefined;
      this.commitIndex = this.shownCommit = undefined;
      this.lastCommit = 0;
      this.commits = [];
      this.restoreFailures = [];
      this.cells.clear();
    });
  }

  /** Fetches the manifest again; the revision may have moved. */
  async refresh(): Promise<Manifest> {
    const { manifest, commit } = await this.fetchManifest();
    this.cells.clear();
    for (const file of manifest.files) this.cells.set(file.path, file);
    this.manifest = manifest;
    this.shownCommit = commit;
    this.restoreFailures = manifest.restoreFailures;
    if (manifest.lens === "perCommit" && manifest.commits > 1) void this.loadCommits();
    else this.commits = [];
    return manifest;
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

  /** Flips one hunk. `revision` is the manifest the caller displays; the backend rejects a stale one. */
  async toggleHunk(revision: number, path: string, hunk: number) {
    await this.review(revision, session.connected().toggleHunk(revision, path, hunk, this.view));
  }

  /** Flips the acknowledgement of one non-text item. */
  async toggleItem(revision: number, path: string, kind: NonTextKind) {
    await this.review(revision, session.connected().toggleItem(revision, path, kind, this.view));
  }

  /** Marks everything of a file seen, or takes every mark back if it all is seen already. */
  async toggleFileSeen(revision: number, path: string) {
    await this.review(revision, session.connected().toggleFileSeen(revision, path, this.view));
  }

  /**
   * Marks the hunks of a file seen against an old base seen again. A range of versions has no such
   * request: its hunks are marked one by one, as the backend flips them.
   */
  async reconfirmFile(revision: number, path: string) {
    if (this.view.type === "changeset") {
      await this.review(revision, session.connected().reconfirmFile(revision, path));
      return;
    }
    const hunks = this.file(path)?.review.hunks ?? [];
    for (const [hunk, state] of hunks.entries()) {
      if (state === "seenAgainstOldBase") await this.toggleHunk(revision, path, hunk);
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
    const revision = this.manifest?.revision;
    if (revision === undefined) return;
    for (const path of this.baseChangedFiles()) {
      // A request that failed refetched the manifest: the rest asks again.
      if (this.manifest?.revision !== revision) return;
      await this.reconfirmFile(revision, path);
    }
  }

  /** The base diff of a hunk seen against an old base; `undefined` after showing why it could not be had. */
  async baseDiff(revision: number, path: string, hunk: number): Promise<BaseDiff | undefined> {
    try {
      return await session.connected().baseDiff(revision, path, hunk);
    } catch (error) {
      this.reportFailure(error, revision);
      return undefined;
    }
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

  /** Puts a changeset on view. The backend pushes `changesetOpened`, which fetches its manifest. */
  async openChangeset(key: ChangesetKey) {
    try {
      await session.connected().openChangeset(key);
    } catch (error) {
      session.notice = errorText(error);
    }
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
  private async review(sent: number, request: Promise<ReviewUpdate>) {
    try {
      this.applyReview(await request);
    } catch (error) {
      await this.refresh().catch(() => undefined);
      this.reportFailure(error, sent);
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
        ? "Changes arrived — try again"
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
      changesetCompletion:
        this.shownCommit === undefined && !this.range ? update.changeset : manifest.changesetCompletion,
    };
    for (const listener of this.listeners) listener(update);
    if (manifest.lens === "perCommit" && manifest.commits > 1) void this.loadCommits();
  }

  /** A mark in the changeset's file, which the view on screen does not show: the changeset's progress and the commits' follow. */
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
