// What the backend sent about the open changeset. Progress and review state are the
// backend's; this holds and distributes them (SPEC I5).

import { untrack } from "svelte";
import { SvelteMap } from "svelte/reactivity";
import type { BaseDiff } from "../generated/BaseDiff";
import type { Base } from "../generated/Base";
import type { Completion } from "../generated/Completion";
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
import { PendingOpen } from "./pendingOpen";
import {
  type ChangesetFacts,
  type FileLayout,
  type FileState,
  type Frame,
  type Parts,
  type ReviewedFile,
  type ViewLayout,
  partsOf,
} from "./reviewModel";
import { ReviewOrder } from "./reviewOrder";
import { type ReviewSource, type WriteListener } from "./reviewSource";
import { session } from "./session.svelte";
import { type Shown, isCurrent } from "./shown";
import { type Range, followed, rangeKey, rangeView } from "./timelineRange";
import { CHANGESET, commitView } from "./view";

/** Said when a request is refused or rejected because the views it came from are about to be replaced. */
const STALE_NOTICE = "Changes arrived — try again";
/** What happens to the changeset on view: another is asked for, announced to be fetched, or the ask came to nothing. */
export type Opening = { type: "requested" } | { type: "opened"; changeset: ChangesetKey } | { type: "unannounced" };

const NO_PATHS: readonly string[] = [];

class ChangesetStore implements ReviewSource {
  /** What the view of the last refresh looks like: its files' structure and its base. Replaced only by a manifest. */
  layout = $state.raw<ViewLayout>();
  /** What is true of the changeset as a whole, whatever view is shown. */
  facts = $state.raw<ChangesetFacts>();
  /** How far the view's files are reviewed. */
  viewCompletion = $state.raw<Completion>();
  /** The base a fetch moved the changeset's to since the last manifest; none in a commit or a range, whose base is their own. */
  private movedBase = $state.raw<Base>();
  /** The view's base as of now. */
  base = $derived(this.movedBase ?? this.layout?.base);
  /** Dependency restores that failed: the analysis of those projects is limited. Pushed as they happen. */
  restoreFailures = $state.raw<RestoreFailure[]>([]);
  /** The current review state of each file of the view, one reactive cell per path: a review update invalidates only its file. */
  private cells = new SvelteMap<string, FileState>();
  private listeners = new Set<WriteListener>();
  /** When each path was last written, in writes counted from the start; never reset, so a path that comes back is newer. */
  private versions = new Map<string, number>();
  private clock = 0;
  /** Which change of the backend the review state held is as of; updates and manifests older than it do not replace it. */
  private order = new ReviewOrder();

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
      const parts = partsOf(manifest, commit);
      const heldFacts = this.facts;
      // A mark answered after the manifest was read is newer than the manifest, whenever the manifest arrives.
      const newer = this.order.load(parts.seq, parts.layout.paths, isCurrent(parts.layout.shown, this.layout?.shown));
      const kept = new Map(newer.files.size ? this.cells : []);
      this.layout = parts.layout;
      this.movedBase = undefined;
      this.viewCompletion = newer.completion && this.viewCompletion ? this.viewCompletion : parts.completion;
      this.facts = newer.changeset && heldFacts ? { ...parts.facts, completion: heldFacts.completion } : parts.facts;
      this.shownCommit = commit;
      this.restoreFailures = manifest.restoreFailures;
      this.write(new Map([...parts.states].map(([path, state]) => [path, kept.get(path) ?? state])), "manifest");
      if (parts.facts.lens === "perCommit" && parts.facts.commits > 1) void this.loadCommits();
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
  /** The range or commit the layout is of, as a string; empty for the changeset. */
  viewKey = $derived(rangeKey(this.layout?.interdiff) + (this.shownCommit === undefined ? "" : `c${this.shownCommit}`));

  /**
   * While the review view shows only the files that had hunks seen against an old base when it was
   * asked for: their paths. Re-confirming does not shrink it, so the view stays as it is.
   */
  baseChanged = $state.raw<ReadonlySet<string>>();

  /** Same array while only review state changes. */
  paths = $derived(this.layout?.paths ?? NO_PATHS);
  private layouts = $derived(new Map(this.layout?.files.map((file) => [file.path, file])));
  /** The files the Changes tab shows: all of them, or those of `baseChanged`. */
  shownFiles = $derived.by(() => {
    const files = this.layout?.files ?? [];
    return this.baseChanged ? files.filter((f) => this.baseChanged?.has(f.path)) : files;
  });

  /** The structure of a file of the view. */
  layoutOf(path: string): FileLayout | undefined {
    return this.layouts.get(path);
  }

  /** The review state of a file of the view as it is now. */
  file(path: string): FileState | undefined {
    return this.cells.get(path);
  }

  /** A file of the view with its review state as it is now; to be read, not kept. */
  reviewed(path: string): ReviewedFile | undefined {
    const layout = this.layouts.get(path);
    const state = this.cells.get(path);
    return layout && state && { ...layout, ...state };
  }

  /** The review state of a file at the moment of use; reads nothing reactively. */
  state(path: string): FileState | undefined {
    return untrack(() => this.cells.get(path));
  }

  version(path: string): number {
    return this.versions.get(path) ?? 0;
  }

  /** The layout of the view on screen, for reading at the moment of use; reads nothing reactively. */
  currentLayout(): ViewLayout | undefined {
    return untrack(() => this.layout);
  }

  /** What is true of the changeset as a whole, for reading at the moment of use; reads nothing reactively. */
  currentFacts(): ChangesetFacts | undefined {
    return untrack(() => this.facts);
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
        else if (event.type === "reviewChanged" && event.revision === this.layout?.shown.revision) {
          if (this.showsChangeset) this.applyReview(event.update);
          else this.applyTotals(event.update);
        } else if (event.type === "reviewStarted" && this.facts) this.facts = { ...this.facts, review: event.review };
        else if (event.type === "restoreFailed") this.restoreFailures = event.failures;
        // Only the changeset's own view has the changeset's base.
        else if (event.type === "baseChanged" && this.showsChangeset) this.movedBase = event.base;
      }),
    );
    session.onResync(() => {
      if (this.layout) this.refresh().catch((error: unknown) => (session.notice = errorText(error)));
    });
    session.onClose(() => this.clear());
  }

  /**
   * Drops everything of the changeset on view, for the views to show that the next one is loading.
   * A manifest fetch underway is of the changeset dropped and counts for nothing.
   */
  private clear() {
    this.manifestLoads.cancel();
    this.layout = this.facts = this.viewCompletion = this.movedBase = undefined;
    this.range = undefined;
    this.baseChanged = undefined;
    this.commitIndex = this.shownCommit = undefined;
    this.lastCommit = 0;
    this.commits = [];
    this.commitsSeq++;
    this.restoreFailures = [];
    this.cells.clear();
    this.order = new ReviewOrder();
  }

  /**
   * The first manifest of a project or of the changeset just opened is being fetched: nothing of the
   * changeset is known, and the views show that instead of what was on view before.
   */
  get loading(): boolean {
    return this.opening || (this.fetching && !this.layout);
  }

  /** The layout on screen is of the changeset as a whole, not of a range or a commit. */
  private get showsChangeset(): boolean {
    return this.layout?.shown.view.type === "changeset";
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
  async refresh(): Promise<Frame | undefined> {
    const loaded = await this.manifestLoads.load(() => this.fetchManifest());
    return loaded && this.layout && this.facts && { layout: this.layout, facts: this.facts };
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
    const to = neighbour(this.shownCommit, delta, this.facts?.commits ?? 0);
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

  /** Shows the changeset as a whole, out of a range and out of the commits (the backend remembers the "All changes" lens), and fetches its manifest. */
  async showWhole() {
    if (this.range) await this.setRange(undefined);
    if (this.facts?.lens === "perCommit") await this.showLens("squash");
    this.commitIndex = undefined;
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
    const follows = this.facts?.target.changeset.type === "workingTree";
    this.range = range && { ...range, live: range.live && follows };
    if (range) this.commitIndex = undefined;
    try {
      await this.refresh();
    } catch (error) {
      session.notice = errorText(error);
    }
  }

  /** What the layout on screen is of, which the views are built for; none before the first manifest. Reads nothing reactively. */
  currentShown(): Shown | undefined {
    return untrack(() => this.layout?.shown);
  }

  /**
   * Whether a request built for `shown` may go out: the manifest is still the one it was built from and
   * no other view has been asked for since. If not, the views it came from are about to be replaced
   * and the viewer is told so.
   */
  admit(shown: Shown): boolean {
    const asked = this.layout && { revision: this.layout.shown.revision, view: this.view };
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
      if (this.layout?.shown.revision !== shown.revision) return;
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

  /** The changeset on view as a whole, whatever range or commit is shown; `undefined` where it could not be read. */
  async wholeParts(): Promise<Parts | undefined> {
    try {
      return partsOf(await session.connected().manifest(CHANGESET), undefined);
    } catch {
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
    const filter = this.facts?.filter;
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
   * Applies the review state a request returned, unless the view it was sent for has been replaced
   * (the manifest of the new one has the state). A rejected request is normally a stale revision:
   * refetch the manifest and tell the viewer to repeat the action.
   */
  private async review(sent: Shown, request: Promise<ReviewUpdate | ReviewUpdate[]>) {
    try {
      const updates = [await request].flat();
      if (isCurrent(sent, this.currentShown())) for (const update of updates) this.applyReview(update);
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
    const current = this.layout?.shown.revision;
    session.notice =
      sent !== undefined && current !== undefined && current !== sent
        ? STALE_NOTICE
        : `Request failed: ${errorText(error)}`;
  }

  /**
   * Applies an update of the view on screen. What the window holds of the file and of the totals is
   * kept if it is as new or newer, so an update that arrives twice (as the answer and as the event)
   * or after a later one changes nothing.
   */
  private applyReview(update: ReviewUpdate) {
    const facts = this.facts;
    if (!this.layout || !facts) return;
    const known = this.cells.has(update.path);
    const fresh = this.order.file(update.path, update.seq);
    const completion = this.order.completion(update.seq);
    // Only the changeset's own view says how far the changeset as a whole is.
    const whole = this.showsChangeset && this.order.changeset(update.seq);
    if (!fresh && !completion && !whole) return;
    if (completion) this.viewCompletion = update.changeset;
    if (whole) this.facts = { ...facts, completion: update.changeset };
    if (fresh && known)
      this.write(new Map([[update.path, { review: update.review, completion: update.completion }]]), "update");
    if (facts.lens === "perCommit" && facts.commits > 1) void this.loadCommits();
  }

  /** A mark in the changeset's file, which a range or commit on screen does not show: the changeset's progress and the commits' follow. */
  private applyTotals(update: ReviewUpdate) {
    if (!this.facts || !this.order.changeset(update.seq)) return;
    this.facts = { ...this.facts, completion: update.changeset };
    void this.loadCommits();
  }

  /**
   * Every write of the files' review state goes through here: the cells take it and the listeners hear
   * which paths changed, or "all" for a manifest, which replaces every file. Review state arrives by
   * answer, event and manifest alike; a listener that reads the cells it is told of follows all three.
   */
  private write(entries: ReadonlyMap<string, FileState>, cause: "update" | "manifest") {
    if (cause === "manifest") this.cells.clear();
    const stamp = ++this.clock;
    for (const [path, file] of entries) {
      this.cells.set(path, file);
      this.versions.set(path, stamp);
    }
    const paths = cause === "manifest" ? "all" : new Set(entries.keys());
    for (const listener of this.listeners) listener(paths);
  }

  /** Calls `listener` after review state was written, with the paths it concerns; returns the unsubscribe function. */
  subscribe(listener: WriteListener): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }
}

export type Changeset = ChangesetStore;

export const changeset = new ChangesetStore();
