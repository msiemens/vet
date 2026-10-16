// What the backend sent about one opened changeset, for as long as it is on view. Progress and review
// state are the backend's; this holds and distributes them (SPEC I5). The session is made when the
// changeset is opened and disposed when another is, or the project closes: its lifetime ends and an
// answer that arrives after that changes nothing.

import { untrack } from "svelte";
import type { BaseDiff } from "../generated/BaseDiff";
import type { Completion } from "../generated/Completion";
import type { ReviewUpdate } from "../generated/ReviewUpdate";
import type { Manifest } from "../generated/Manifest";
import type { RestoreFailure } from "../generated/RestoreFailure";
import type { ReviewFilter } from "../generated/ReviewFilter";
import type { NonTextKind } from "../generated/NonTextKind";
import type { ChangesetKey } from "../generated/ChangesetKey";
import type { CommentAt } from "../generated/CommentAt";
import type { View } from "../generated/View";
import type { CommitUnit } from "../generated/CommitUnit";
import type { Lens } from "../generated/Lens";
import { STALE_NOTICE } from "../lib/notice";
import { pathsBelow } from "../lib/paths";
import { isKind } from "../lib/requestError";
import type { SessionEvent } from "./backend";
import { CommentDrafts, emptyChangesetComment, unsentComments } from "./commentDrafts";
import { CommitList } from "./commitList";
import { type SessionFindSets, newFindSets } from "./findSets.svelte";
import { commitToShow, neighbour } from "./commitLens";
import { type SessionChecks, newChecks } from "./checks.svelte";
import { Lifetime } from "./lifetime";
import { notices } from "./notices.svelte";
import { type SessionPullRequest, newPullRequest } from "./pullRequest.svelte";
import { Latest, Resource } from "./resource";
import {
  type ChangesetFacts,
  type FileLayout,
  type FileState,
  type Frame,
  type ReviewedFile,
  type ViewLayout,
  partsOf,
} from "./reviewModel";
import { newReviewCells } from "./reviewCells.svelte";
import { reportFailure, reviewMarks } from "./reviewMarks";
import { whileReviewShown } from "./reviewFollow";
import { type SessionReviewChanges, newReviewChanges } from "./reviewChanges.svelte";
import { type WriteListener } from "./reviewSource";
import { type SessionPipelines, newPipelines } from "./pipelines.svelte";
import { type SessionVersions, newVersions } from "./versions.svelte";
import { session } from "./session.svelte";
import { type Shown, isCurrent } from "./shown";
import { type SessionThreads, newThreads } from "./threads.svelte";
import type { ViewSource } from "./viewScope.svelte";
import { type Range, normalised, rangeView } from "./timelineRange";
import { commitView } from "./view";
import { type ViewChange, type ViewPoint, viewChange, viewPoint } from "./viewChange";
import { refetchesWhole } from "./wholeFetch";
import { type WholeView, newWhole } from "./wholeView.svelte";

const NO_PATHS: readonly string[] = [];
/** What a session is made from. */
export interface SessionStart {
  /** The changeset the session is of, fixed for its life. */
  key: ChangesetKey;
  /** The manifest of the changeset the opener has read already: the session's first read takes it instead of asking again. */
  manifest?: Manifest;
  /** What the views last were told about, which the first change is told against. */
  viewed?: ViewPoint;
}

class Session implements ViewSource {
  readonly key: ChangesetKey;
  readonly lifetime = new Lifetime();
  /** A manifest read ahead of the session's own, used by its first read of the changeset view. */
  private seed?: Manifest;
  /** What the view of the last refresh looks like: its files' structure and its base. Replaced only by a manifest. */
  layout = $state.raw<ViewLayout>();
  /** What is true of the changeset as a whole, whatever view is shown. */
  facts = $state.raw<ChangesetFacts>();
  /** The changeset as a whole: its files and their review state, whatever range or commit the view on screen shows. */
  readonly whole: WholeView = newWhole(() => !this.lifetime.alive);
  /** The whole-changeset view was fed by the refresh underway; it needs no read of its own then. */
  private fed = false;
  /** The comment threads, one list per view: the changeset's and the range on view. */
  readonly threads: SessionThreads;
  /** The versions of the open review; read while the review screen is shown. */
  readonly versions: SessionVersions = newVersions(() => session.connected());
  /** What differs since a version of the review; the queue screen shows it too, so it lasts as long as the session. */
  readonly reviewChanges: SessionReviewChanges = newReviewChanges(() => session.connected());
  /** The Pipelines tool's choice, folds and steps; the choice and folds last as long as the session. */
  readonly pipelines: SessionPipelines = newPipelines(() => session.backend);
  /** What the platform says of the pull request; none for a branch or the working tree, which never ask. */
  readonly pullRequest: SessionPullRequest | undefined;
  /** The checks of the pull request's commits, as the backend pushes them. */
  readonly checks: SessionChecks = newChecks();
  /** The Find tool's result sets. */
  readonly findSets: SessionFindSets = newFindSets(this);
  /** Find in Files' place searched (`scopeChoices`'s id) and chosen result, for the dialog to come back to; the other fields stay with the window. */
  readonly findInFiles = $state<{ scope?: string; selected?: string }>({});
  /** What the cards' comment forms hold that was not sent. */
  readonly drafts = new CommentDrafts();
  /** The comment on the changeset as a whole being written in the Comments tool window. */
  readonly changesetComment = $state(emptyChangesetComment());
  /** How far the view's files are reviewed. */
  viewCompletion = $state.raw<Completion>();
  /** Dependency restores that failed: the analysis of those projects is limited. Pushed as they happen. */
  restoreFailures = $state.raw<RestoreFailure[]>([]);
  /**
   * The current review state of each file of the view, one reactive cell per path: a review update invalidates
   * only its file. It knows which change of the backend the state is as of; updates and manifests older than
   * it do not replace it.
   */
  private cells = newReviewCells();
  /** The requests that mark, as the view on screen sends them. */
  private marks = reviewMarks({
    backend: () => session.connected(),
    admit: (shown) => this.admit(shown),
    current: () => this.currentShown(),
    apply: (update) => this.applyReview(update),
    refresh: () => this.refresh(),
    hunks: (path) => this.cells.state(path)?.review.hunks ?? [],
    ended: () => !this.lifetime.alive,
  });

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
  /** The commits with their own progress, oldest first; loaded only while the review screen shows the changeset by commit. */
  commits = $state.raw<CommitUnit[]>([]);
  /** The commits by revision, held in one place: this store, a tab at a commit and Find in Files read from here. */
  readonly commitList = new CommitList(() => session.connected().commits());
  /** The per-commit reads the store started; only the latest may set the list it shows. */
  private commitReads = new Latest();
  /** Whether the review screen is shown (see `showReview`): only it reads the per-commit list, the range's threads, the versions and the Pipelines steps. */
  private reviewShown = false;
  /** A manifest is being fetched. */
  private fetching = $state(false);
  private manifestLoads = new Resource<{ manifest: Manifest; commit: number | undefined }>({
    busy: (on) => (this.fetching = on),
    apply: ({ manifest, commit }) => {
      const parts = partsOf(manifest, commit);
      const heldFacts = this.facts;
      // A mark answered after the manifest was read is newer than the manifest, whenever the manifest arrives.
      const newer = this.cells.order.load(
        parts.seq,
        parts.layout.paths,
        isCurrent(parts.layout.shown, this.layout?.shown),
      );
      this.layout = parts.layout;
      this.viewCompletion = newer.completion && this.viewCompletion ? this.viewCompletion : parts.completion;
      this.facts = newer.changeset && heldFacts ? { ...parts.facts, completion: heldFacts.completion } : parts.facts;
      this.shownCommit = commit;
      this.threads.viewed(parts.layout.interdiff);
      this.restoreFailures = manifest.restoreFailures;
      this.cells.replace(parts.states, newer);
      if (parts.facts.lens === "perCommit" && parts.facts.commits > 1) void this.loadCommits();
      else this.commits = [];
      this.announce();
    },
  });
  /** What requests about the files read: the interdiff of `range`, a commit, or the changeset. */
  get view(): View {
    return this.range || this.commitIndex === undefined ? rangeView(this.range) : commitView(this.commitIndex);
  }

  /**
   * While the review view shows only the files that had hunks seen against an old base when it was
   * asked for: their paths. Re-confirming does not shrink it, so the view stays as it is.
   */
  private limitedTo = $state.raw<ReadonlySet<string>>();
  get baseChanged(): ReadonlySet<string> | undefined {
    return this.limitedTo;
  }

  /** What the views last told about, which the next change is told against; the next session takes it over. */
  viewed?: ViewPoint;
  private viewListeners = new Set<(frame: Frame, change: ViewChange) => void>();

  /** Same array while only review state changes. */
  paths = $derived(this.layout?.paths ?? NO_PATHS);
  private layouts = $derived(new Map(this.layout?.files.map((file) => [file.path, file])));
  /** The files the Changes tab shows: all of them, or those of `baseChanged`. */
  shownFiles = $derived.by(() => {
    const files = this.layout?.files ?? [];
    return this.baseChanged ? files.filter((f) => this.baseChanged?.has(f.path)) : files;
  });

  /** The head commit of a pull request, whose checks the toolbar shows; none for another changeset. */
  get headCommit(): string | undefined {
    const facts = this.facts;
    return facts?.target.changeset.type === "pullRequest" ? facts.target.head : undefined;
  }

  /** The commit whose runs the Pipelines tool shows: the one on view by commit, else the head. */
  get pipelineCommit(): string | undefined {
    return this.shownCommit === undefined ? this.headCommit : this.commits[this.shownCommit]?.commit.id;
  }

  /** How many files have hunks seen against an older base, which "review only these" narrows to. */
  get baseChangedCount(): number {
    return this.baseChanged ? this.baseChangedFiles().length : 0;
  }

  /** Whether a review of the changeset is open (SPEC §6: Finish review ends it). */
  get reviewOpen(): boolean {
    return this.facts?.review.state === "open";
  }

  /** The lens the changeset is reviewed in; none before the first manifest. */
  get lens(): Lens | undefined {
    return this.facts?.lens;
  }

  /** How many commits the changeset has. */
  get commitCount(): number {
    return this.facts?.commits ?? 0;
  }

  /** The commit on view, with its own progress; none for the whole changeset. */
  get shownCommitUnit(): CommitUnit | undefined {
    return this.shownCommit === undefined ? undefined : this.commits[this.shownCommit];
  }

  /** The progress the header shows: the commit on view's own, else the view's. */
  get shownCompletion(): Completion | undefined {
    return this.shownCommit === undefined ? this.viewCompletion : (this.facts?.completion ?? this.viewCompletion);
  }

  /** The structure of a file of the view. */
  layoutOf(path: string): FileLayout | undefined {
    return this.layouts.get(path);
  }

  /** The review state of a file of the view as it is now. */
  file(path: string): FileState | undefined {
    return this.cells.file(path);
  }

  /** A file of the view with its review state as it is now; to be read, not kept. */
  reviewed(path: string): ReviewedFile | undefined {
    const layout = this.layouts.get(path);
    const state = this.cells.file(path);
    return layout && state && { ...layout, ...state };
  }

  /** The review state of a file at the moment of use; reads nothing reactively. */
  state(path: string): FileState | undefined {
    return this.cells.state(path);
  }

  version(path: string): number {
    return this.cells.version(path);
  }

  /** The layout of the view on screen, for reading at the moment of use; reads nothing reactively. */
  currentLayout(): ViewLayout | undefined {
    return untrack(() => this.layout);
  }

  /** What is true of the changeset as a whole, for reading at the moment of use; reads nothing reactively. */
  currentFacts(): ChangesetFacts | undefined {
    return untrack(() => this.facts);
  }

  /**
   * The whole changeset's files and base with what is true of it, whatever range or commit the review
   * shows: what Problems and the queue panel read.
   */
  get wholeFrame(): Frame | undefined {
    const layout = this.whole.layout;
    return layout && this.facts && { layout, facts: this.facts };
  }

  /**
   * The frame on view, reading the manifest only when the store holds none: the review screen is
   * rebuilt from the store, which keeps the manifest current for the queue too.
   */
  async frame(): Promise<Frame | undefined> {
    if (this.layout && this.facts) return { layout: this.layout, facts: this.facts };
    return this.refresh();
  }

  constructor(start: SessionStart) {
    this.key = start.key;
    this.seed = start.manifest;
    this.viewed = start.viewed;
    this.threads = newThreads({
      backend: () => session.connected(),
      ended: () => !this.lifetime.alive,
      admit: (shown) => this.admit(shown),
      reportFailure: (error, sent) => this.reportFailure(error, sent),
    });
    this.lifetime.add(() => {
      this.clear();
      this.viewListeners.clear();
    });
    this.pullRequest =
      start.key.type === "pullRequest" ? newPullRequest({ backend: () => session.connected() }) : undefined;
    this.lifetime.add(() => this.pullRequest?.clear());
    this.lifetime.add(() => this.reviewChanges.end());
    this.lifetime.add(() => this.findSets.end());
    this.lifetime.add(session.connected().onAccountsChanged(() => this.pipelines.accountsChanged()));
    whileReviewShown(this.lifetime, () => this.showReview());
  }

  /** The changeset is on view: the manifest is read, for the queue as well as the review screen. `revision` is the one the backend announced, if it did. */
  start(revision?: number) {
    void this.reread(revision);
    void this.threads.reload();
    void this.pullRequest?.reload();
  }

  /** The connection came back: everything is read again. `manifest` is the changeset's, if the opener read it already. */
  resync(manifest?: Manifest) {
    this.seed = manifest;
    void this.reread(undefined);
    void this.threads.reload();
    void this.pullRequest?.reload();
    this.versions.resync();
    this.reviewChanges.resync();
  }

  /** How many comment boxes hold text that was not added as a comment, which ending the session loses. */
  unsentComments(): number {
    return unsentComments(this.drafts, this.changesetComment);
  }

  /** Ends the session: what it started counts for nothing from now on. */
  dispose() {
    this.lifetime.dispose();
  }

  /**
   * What only the review screen shows is held while it is shown: the per-commit list, the range's
   * threads, the versions and the Pipelines steps. Returns what lets go
   * of them when the screen goes away or the session ends.
   */
  private showReview(): () => void {
    this.reviewShown = true;
    this.threads.follow();
    this.versions.follow();
    if (this.facts?.lens === "perCommit" && this.facts.commits > 1) void this.loadCommits();
    return () => {
      this.reviewShown = false;
      this.threads.unfollow();
      this.versions.unfollow();
      this.pipelines.hideSteps();
      this.commitReads.cancel();
      this.commits = [];
    };
  }

  /** An event of the project, other than the announcement of a changeset. */
  hear(event: SessionEvent) {
    if (!this.lifetime.alive) return;
    this.versions.hear(event);
    this.reviewChanges.hear(event);
    this.pullRequest?.hear(event);
    this.checks.hear(event);
    if (event.type === "snapshotSettled") {
      void this.reread(event.revision);
      void this.threads.reload();
    } else if (event.type === "threadsChanged") void this.threads.reload();
    else if (event.type === "targetChanged") void this.reread(undefined);
    else if (event.type === "lensChanged") void this.followLens(event.lens);
    // A mark made in a range or a commit is announced as that of the changeset's file, which they do not show.
    else if (event.type === "reviewChanged" && event.revision === this.layout?.shown.revision) {
      if (this.showsChangeset) this.applyReview(event.update);
      else this.applyTotals(event.update);
      this.whole.heard(event.update, event.revision);
    } else if (event.type === "reviewStarted" && this.facts) this.facts = { ...this.facts, review: event.review };
    else if (event.type === "restoreFailed") this.restoreFailures = event.failures;
  }

  /**
   * Drops everything of the changeset, for the views to show that the next one is loading.
   * A manifest fetch underway is of the changeset dropped and counts for nothing.
   */
  private clear() {
    this.manifestLoads.cancel();
    this.layout = this.facts = this.viewCompletion = undefined;
    this.range = undefined;
    this.limitedTo = undefined;
    this.commitIndex = this.shownCommit = undefined;
    this.lastCommit = 0;
    this.commits = [];
    this.commitReads.cancel();
    this.restoreFailures = [];
    this.cells.reset();
    this.whole.clear();
    this.threads.clear();
  }

  /** The first manifest of the changeset is being fetched: nothing of it is known yet. */
  get loading(): boolean {
    return this.fetching && !this.layout;
  }

  /** The layout on screen is of the changeset as a whole, not of a range or a commit. */
  private get showsChangeset(): boolean {
    return this.layout?.shown.view.type === "changeset";
  }

  /**
   * Called with the frame on view when the views it shows changed: a newer revision, another range or
   * commit, or another filter. The listener is told which (see `viewChange`). Returns the unsubscribe function.
   */
  onViewChange(listener: (frame: Frame, change: ViewChange) => void): () => void {
    this.viewListeners.add(listener);
    return () => this.viewListeners.delete(listener);
  }

  /** Tells the listeners what changed about the frame on view since the last time. */
  private announce() {
    const { layout, facts } = this;
    if (!layout || !facts || !this.lifetime.alive) return;
    const point = viewPoint(layout, this.shownCommit, this.limitedTo);
    const change = viewChange(this.viewed, point);
    this.viewed = point;
    if (change === "none") return;
    for (const listener of this.viewListeners) listener({ layout, facts }, change);
  }

  private limit(files: ReadonlySet<string> | undefined) {
    this.limitedTo = files;
    this.announce();
  }

  /**
   * Fetches the manifest again; the revision may have moved. Only the latest fetch counts: an older
   * one that settles later gives the latest's manifest. Resolves with nothing when the project was
   * closed meanwhile.
   */
  async refresh(): Promise<Frame | undefined> {
    if (!this.lifetime.alive) return undefined;
    const loaded = await this.manifestLoads.load(() => this.fetchManifest());
    return loaded && this.layout && this.facts ? { layout: this.layout, facts: this.facts } : undefined;
  }

  /**
   * Reads the manifest of the view on screen again, and the changeset as a whole when the view is not
   * it: a settle, a target change, a resync or another changeset. A mark is written into the whole
   * view by its own announcement instead, so a burst of marks reads nothing.
   */
  private async reread(revision: number | undefined) {
    this.fed = false;
    try {
      await this.refresh();
    } catch (error) {
      if (this.lifetime.alive) notices.report(error);
    }
    this.seed = undefined;
    if (!this.lifetime.alive || this.wholeFed || this.view.type === "changeset") return;
    if (refetchesWhole(revision, this.whole.revision)) await this.whole.reload();
  }

  private takeSeed(): Manifest | undefined {
    const seed = this.seed;
    this.seed = undefined;
    return seed;
  }

  /** Whether the refresh that just ran fed the whole-changeset view; read after an await, so not narrowed. */
  private get wholeFed(): boolean {
    return this.fed;
  }

  /**
   * The manifest of the view. A range that covers every push becomes the changeset, which only the
   * manifest's latest version says, and the commit the lens asks for is shown instead of the changeset, as the
   * manifest says too. A range or commit the backend does not find (its versions or commit are gone) gives
   * way to the changeset.
   */
  private async fetchManifest(): Promise<{ manifest: Manifest; commit: number | undefined }> {
    const backend = session.connected();
    for (let attempt = 0; ; attempt++) {
      const range = this.range;
      const commit = range ? undefined : this.commitIndex;
      let manifest: Manifest;
      try {
        const seed = !range && commit === undefined ? this.takeSeed() : undefined;
        manifest = seed ?? (await backend.manifest(this.view));
        this.lifetime.check();
      } catch (error) {
        this.lifetime.check();
        if ((!range && commit === undefined) || !isKind(error, "notFound")) throw error;
        if (range) this.range = undefined;
        else this.commitIndex = undefined;
        notices.report(error);
        continue;
      }
      const latest = manifest.review.state === "open" ? manifest.review.version : undefined;
      if (range && normalised(range, latest) === undefined && attempt < 3) {
        this.range = undefined;
        continue;
      }
      // The changeset manifest is the changeset as a whole, whether the view settles on it or moves on
      // to the commit the lens asks for: it is held for Problems, the queue panel and a whole tab.
      if (!range && commit === undefined) {
        this.whole.adopt(partsOf(manifest, undefined));
        this.fed = true;
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
    if (!this.reviewShown || !this.lifetime.alive) return;
    const revision = this.layout?.shown.revision;
    if (revision === undefined) return;
    const isLatest = this.commitReads.begin();
    try {
      const { revision: answered, units } = await this.commitList.reload();
      // A list of a revision the view moved on from says nothing of the one on screen.
      if (isLatest() && answered === revision) this.commits = units;
    } catch (error) {
      if (isLatest()) notices.failed("Could not load the commits", error);
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
      notices.report(error);
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
      notices.report(error);
    }
  }

  /** The changeset is reviewed in `lens` now, as this window or another chose: shows the view that goes with it. */
  private async followLens(lens: Lens) {
    // The commits replace a range; leaving them for a range keeps it, already on view.
    if (lens === "perCommit") this.range = undefined;
    else if (this.range) return;
    this.commitIndex = lens === "perCommit" ? this.lastCommit : undefined;
    try {
      await this.refresh();
    } catch (error) {
      notices.report(error);
    }
  }

  /** Shows the changeset as a whole, out of a range and out of the commits (the backend remembers the "All changes" lens), and fetches its manifest. */
  async showWhole() {
    if (this.range) await this.setRange(undefined);
    if (this.facts?.lens === "perCommit") await this.showLens("squash");
    if (!this.lifetime.alive) return;
    this.commitIndex = undefined;
    try {
      await this.refresh();
    } catch (error) {
      notices.report(error);
    }
  }

  /**
   * Shows the interdiff of `range`, or the whole changeset for none, and fetches its manifest. It
   * leaves the by commit lens, which the range replaces.
   */
  async setRange(range: Range | undefined) {
    const leavingCommits = this.facts?.lens === "perCommit";
    if (range) this.limit(undefined);
    this.range = range;
    if (range) this.commitIndex = undefined;
    // With the range set, the lens change it causes shows nothing new and loads nothing.
    if (leavingCommits) await this.showLens("squash");
    try {
      await this.refresh();
    } catch (error) {
      notices.report(error);
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
    notices.show(STALE_NOTICE);
    return false;
  }

  /** Flips one hunk of the file in `shown`, the view the caller displays; the backend rejects a stale revision. */
  toggleHunk(shown: Shown, path: string, hunk: number) {
    return this.marks.toggleHunk(shown, path, hunk);
  }

  /** Flips the flag of one hunk. */
  toggleFlag(shown: Shown, path: string, hunk: number) {
    return this.marks.toggleFlag(shown, path, hunk);
  }

  /** Flips the acknowledgement of one non-text item. */
  toggleItem(shown: Shown, path: string, kind: NonTextKind) {
    return this.marks.toggleItem(shown, path, kind);
  }

  /** Marks everything of a file seen, or takes every mark back if it all is seen already. */
  toggleFileSeen(shown: Shown, path: string) {
    return this.marks.toggleFileSeen(shown, path);
  }

  /** Marks everything of every file of the view below the directory `dir` seen, or takes every mark back. */
  async setDirectorySeen(shown: Shown, dir: string, seen: boolean) {
    const paths = pathsBelow(this.paths, dir);
    if (paths.length) await this.marks.setFilesSeen(shown, paths, seen);
  }

  /** Marks the hunks of a file seen against an old base seen again. */
  reconfirmFile(shown: Shown, path: string) {
    return this.marks.reconfirmFile(shown, path);
  }

  /** The files that have hunks seen against an old base, as they are now. */
  baseChangedFiles(): string[] {
    return this.paths.filter((path) => this.file(path)?.review.hunks.includes("seenAgainstOldBase"));
  }

  /** Shows only the files with hunks seen against an old base in the review view, which then shows the changeset. */
  async showBaseChanged() {
    if (this.range) await this.setRange(undefined);
    if (!this.lifetime.alive) return;
    this.limit(new Set(this.baseChangedFiles()));
  }

  /** Shows every file again. */
  showAll() {
    this.limit(undefined);
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
  baseDiff(shown: Shown, path: string, hunk: number): Promise<BaseDiff | undefined> {
    return this.marks.baseDiff(shown, path, hunk);
  }

  /** Where a comment on a part of the file attaches in `shown`; rejects if `shown` is not current. */
  commentTarget(shown: Shown, path: string, at: CommentAt) {
    return this.marks.commentTarget(shown, path, at);
  }

  /**
   * Ends the open review and says how it ended. The backend pushes `snapshotSettled`, which
   * fetches the manifest again, back to browsing.
   */
  async finishReview() {
    try {
      this.range = undefined;
      const complete = await session.connected().finishReview();
      notices.show(complete ? "Review finished: everything was seen" : "Review finished with unseen changes left");
    } catch (error) {
      notices.report(error);
    }
  }

  /** Limits this review to the folder `path`; the project's filter stays as it is. */
  async reviewOnly(path: string) {
    const filter = this.facts?.filter;
    if (!filter) return;
    await this.narrow({ ...filter, roots: [path] });
  }

  /**
   * Narrows this review to `filter` in place of the project's, or ends the narrowing for none. As with `setFilter`,
   * the backend then pushes `snapshotSettled`.
   */
  async narrow(filter: ReviewFilter | undefined) {
    const changeset = this.facts?.target.changeset;
    if (!changeset) return;
    try {
      await session.connected().narrowReview(changeset, filter);
    } catch (error) {
      notices.report(error);
    }
  }

  /**
   * Tells the viewer why a request failed. When the manifest has moved on from the revision the
   * request was sent for, the views are about to be replaced and the action has to be repeated.
   */
  reportFailure(error: unknown, sent?: number) {
    reportFailure(error, sent, this.layout?.shown.revision);
  }

  /** An answer another view of this window got for the view on screen's own lines (a tab at the commit on screen): it is as good as one this store asked for. */
  adopt(shown: Shown, update: ReviewUpdate) {
    if (isCurrent(shown, this.currentShown())) this.applyReview(update);
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
    const fresh = this.cells.order.file(update.path, update.seq);
    const completion = this.cells.order.completion(update.seq);
    // Only the changeset's own view says how far the changeset as a whole is.
    const whole = this.showsChangeset && this.cells.order.changeset(update.seq);
    if (!fresh && !completion && !whole) return;
    if (completion) this.viewCompletion = update.changeset;
    if (whole) this.facts = { ...facts, completion: update.changeset };
    if (fresh && known) this.cells.put(update.path, { review: update.review, completion: update.completion });
    if (facts.lens === "perCommit" && facts.commits > 1) void this.loadCommits();
  }

  /** A mark in the changeset's file, which a range or commit on screen does not show: the changeset's progress and the commits' follow. */
  private applyTotals(update: ReviewUpdate) {
    if (!this.facts || !this.cells.order.changeset(update.seq)) return;
    this.facts = { ...this.facts, completion: update.changeset };
    void this.loadCommits();
  }

  /** Calls `listener` after review state was written, with the paths it concerns; returns the unsubscribe function. */
  subscribe(listener: WriteListener): () => void {
    return this.cells.subscribe(listener);
  }
}

export type ChangesetSession = Session;

export const newSession = (start: SessionStart): ChangesetSession => new Session(start);
