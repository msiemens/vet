// The changeset on view as the window's stores and views read it, forwarded to the session of the changeset
// that is on view. Before the first changeset is read and while another loads there is no session, and
// the forwarding object answers as an empty store does. Temporary: the views read the session themselves.

import { untrack } from "svelte";
import type { BaseDiff } from "../generated/BaseDiff";
import type { Completion } from "../generated/Completion";
import type { CommentAt } from "../generated/CommentAt";
import type { CommitUnit } from "../generated/CommitUnit";
import type { Lens } from "../generated/Lens";
import type { NonTextKind } from "../generated/NonTextKind";
import type { ChangesetKey } from "../generated/ChangesetKey";
import type { RestoreFailure } from "../generated/RestoreFailure";
import type { ReviewFilter } from "../generated/ReviewFilter";
import type { View } from "../generated/View";
import { STALE_NOTICE } from "../lib/notice";
import { RequestError } from "../lib/requestError";
import type { ChangesetSession } from "./changesetSession.svelte";
import { CommitList } from "./commitList";
import { notices } from "./notices.svelte";
import { opened } from "./opened.svelte";
import {
  changesetLines,
  excludePath,
  fetchRemotes,
  folders,
  listChangesets,
  listDirectory,
  listPullRequests,
  patternMatches,
  setFilter,
  setProjectTrusted,
  trustPullRequest,
} from "./projectRequests";
import { resubscribing } from "./resubscribing";
import type { Answer } from "./reviewMarks";
import { session as connection } from "./session.svelte";
import type { ChangesetFacts, FileLayout, FileState, Frame, ReviewedFile, ViewLayout } from "./reviewModel";
import type { WriteListener } from "./reviewSource";
import type { Shown } from "./shown";
import { type Range, rangeKey, rangeView } from "./timelineRange";
import type { ViewChange } from "./viewChange";
import type { ViewSource } from "./viewScope.svelte";
import type { WholeView } from "./wholeView.svelte";

const NO_PATHS: readonly string[] = [];
const NO_FAILURES: RestoreFailure[] = [];
const NO_COMMITS: CommitUnit[] = [];

/** Listeners given to the forwarding objects stay on whichever session is on view. */
const onView = <L>(attach: (session: ChangesetSession, listener: L) => () => void) =>
  resubscribing<ChangesetSession, L>(
    { now: () => opened.session, onReplaced: (listener) => void opened.onSession(listener) },
    attach,
  );

/** A request about a view that has no session: the views it came from are gone. */
function gone(): Promise<void> {
  notices.show(STALE_NOTICE);
  return Promise.resolve();
}

/** The part of a view that file tabs read, forwarded to what `target` is now. */
abstract class ForwardedView implements ViewSource {
  protected abstract get target(): ViewSource | undefined;
  protected abstract attach(session: ChangesetSession, listener: WriteListener): () => void;
  private watch = onView<WriteListener>((session, listener) => this.attach(session, listener));

  currentShown(): Shown | undefined {
    return untrack(() => this.target)?.currentShown();
  }

  currentLayout(): ViewLayout | undefined {
    return untrack(() => this.target)?.currentLayout();
  }

  layoutOf(path: string): FileLayout | undefined {
    return this.target?.layoutOf(path);
  }

  state(path: string): FileState | undefined {
    return untrack(() => this.target)?.state(path);
  }

  version(path: string): number {
    return untrack(() => this.target)?.version(path) ?? 0;
  }

  subscribe(listener: WriteListener): () => void {
    return this.watch(listener);
  }

  admit(shown: Shown): boolean {
    const target = untrack(() => this.target);
    if (target) return target.admit(shown);
    notices.show(STALE_NOTICE);
    return false;
  }

  toggleHunk(shown: Shown, path: string, hunk: number): Promise<void> {
    return this.target?.toggleHunk(shown, path, hunk) ?? gone();
  }

  toggleFlag(shown: Shown, path: string, hunk: number): Promise<void> {
    return this.target?.toggleFlag(shown, path, hunk) ?? gone();
  }

  toggleItem(shown: Shown, path: string, kind: NonTextKind): Promise<void> {
    return this.target?.toggleItem(shown, path, kind) ?? gone();
  }

  toggleFileSeen(shown: Shown, path: string): Promise<void> {
    return this.target?.toggleFileSeen(shown, path) ?? gone();
  }

  reconfirmFile(shown: Shown, path: string): Promise<void> {
    return this.target?.reconfirmFile(shown, path) ?? gone();
  }

  async baseDiff(shown: Shown, path: string, hunk: number): Promise<BaseDiff | undefined> {
    const target = this.target;
    if (target) return target.baseDiff(shown, path, hunk);
    await gone();
    return undefined;
  }

  commentTarget(shown: Shown, path: string, at: CommentAt) {
    const target = this.target;
    if (target) return target.commentTarget(shown, path, at);
    notices.show(STALE_NOTICE);
    return Promise.reject(new RequestError("outdated", STALE_NOTICE));
  }
}

/** The changeset as a whole, whatever range or commit the review shows. */
class ForwardedWhole extends ForwardedView {
  protected get target(): WholeView | undefined {
    return opened.session?.whole;
  }

  protected attach(session: ChangesetSession, listener: WriteListener) {
    return session.whole.subscribe(listener);
  }

  get layout(): ViewLayout | undefined {
    return this.target?.layout;
  }

  get revision(): number | undefined {
    return this.target?.revision;
  }

  reviewed(path: string): ReviewedFile | undefined {
    return this.target?.reviewed(path);
  }

  heard(update: Answer, revision: number) {
    this.target?.heard(update, revision);
  }
}

class ForwardedChangeset extends ForwardedView {
  protected get target(): ChangesetSession | undefined {
    return opened.session;
  }

  protected attach(session: ChangesetSession, listener: WriteListener) {
    return session.subscribe(listener);
  }

  readonly whole = new ForwardedWhole();

  get layout(): ViewLayout | undefined {
    return opened.session?.layout;
  }

  get facts(): ChangesetFacts | undefined {
    return opened.session?.facts;
  }

  get viewCompletion(): Completion | undefined {
    return opened.session?.viewCompletion;
  }

  get restoreFailures(): RestoreFailure[] {
    return opened.session?.restoreFailures ?? NO_FAILURES;
  }

  get range(): Range | undefined {
    return opened.session?.range;
  }

  get commitIndex(): number | undefined {
    return opened.session?.commitIndex;
  }

  get shownCommit(): number | undefined {
    return opened.session?.shownCommit;
  }

  get commits(): CommitUnit[] {
    return opened.session?.commits ?? NO_COMMITS;
  }

  private noCommits = new CommitList(() => connection.connected().commits());

  /** The commits by revision; the list of the session on view. */
  get commitList(): CommitList {
    return opened.session?.commitList ?? this.noCommits;
  }

  get view(): View {
    return opened.session?.view ?? rangeView(undefined);
  }

  get viewKey(): string {
    return opened.session?.viewKey ?? rangeKey(undefined);
  }

  get baseChanged(): ReadonlySet<string> | undefined {
    return opened.session?.baseChanged;
  }

  get paths(): readonly string[] {
    return opened.session?.paths ?? NO_PATHS;
  }

  get shownFiles() {
    return opened.session?.shownFiles ?? [];
  }

  get headCommit(): string | undefined {
    return opened.session?.headCommit;
  }

  get pipelineCommit(): string | undefined {
    return opened.session?.pipelineCommit;
  }

  get baseChangedCount(): number {
    return opened.session?.baseChangedCount ?? 0;
  }

  get reviewOpen(): boolean {
    return opened.session?.reviewOpen ?? false;
  }

  get lens(): Lens | undefined {
    return opened.session?.lens;
  }

  get commitCount(): number {
    return opened.session?.commitCount ?? 0;
  }

  get shownCommitUnit(): CommitUnit | undefined {
    return opened.session?.shownCommitUnit;
  }

  get shownCompletion(): Completion | undefined {
    return opened.session?.shownCompletion;
  }

  get wholeFrame(): Frame | undefined {
    return opened.session?.wholeFrame;
  }

  /** A changeset is being read and nothing of it is known yet. */
  get loading(): boolean {
    return opened.loading;
  }

  file(path: string): FileState | undefined {
    return opened.session?.file(path);
  }

  reviewed(path: string): ReviewedFile | undefined {
    return opened.session?.reviewed(path);
  }

  currentFacts(): ChangesetFacts | undefined {
    return untrack(() => opened.session)?.currentFacts();
  }

  /** The frame on view, read once a session is there to hold it. */
  async frame(): Promise<Frame | undefined> {
    return (await opened.current())?.frame();
  }

  readonly onOpening = (listener: Parameters<typeof opened.onOpening>[0]) => opened.onOpening(listener);

  readonly onViewChange = onView<(frame: Frame, change: ViewChange) => void>((session, listener) =>
    session.onViewChange(listener),
  );

  openChangeset(key: ChangesetKey) {
    return opened.open(key);
  }

  async refresh(): Promise<Frame | undefined> {
    return (await opened.current())?.refresh();
  }

  async loadCommits() {
    await opened.session?.loadCommits();
  }

  async showCommit(index: number) {
    await opened.session?.showCommit(index);
  }

  stepCommit(delta: number): boolean {
    return opened.session?.stepCommit(delta) ?? false;
  }

  async showLens(lens: Lens) {
    await opened.session?.showLens(lens);
  }

  async showWhole() {
    await opened.session?.showWhole();
  }

  async setRange(range: Range | undefined) {
    await opened.session?.setRange(range);
  }

  async setDirectorySeen(shown: Shown, dir: string, seen: boolean) {
    await (opened.session?.setDirectorySeen(shown, dir, seen) ?? gone());
  }

  baseChangedFiles(): string[] {
    return opened.session?.baseChangedFiles() ?? [];
  }

  async showBaseChanged() {
    await opened.session?.showBaseChanged();
  }

  showAll() {
    opened.session?.showAll();
  }

  async reconfirmAll() {
    await opened.session?.reconfirmAll();
  }

  async finishReview() {
    await opened.session?.finishReview();
  }

  async reviewOnly(path: string) {
    await opened.session?.reviewOnly(path);
  }

  async narrow(filter: ReviewFilter | undefined) {
    await opened.session?.narrow(filter);
  }

  reportFailure(error: unknown, sent?: number) {
    opened.session?.reportFailure(error, sent);
  }

  adopt(shown: Shown, update: Parameters<ChangesetSession["adopt"]>[1]) {
    opened.session?.adopt(shown, update);
  }

  readonly listChangesets = listChangesets;
  readonly listPullRequests = listPullRequests;
  readonly changesetLines = changesetLines;
  readonly fetch = fetchRemotes;
  readonly setFilter = setFilter;
  readonly patternMatches = patternMatches;
  readonly folders = folders;
  readonly setProjectTrusted = setProjectTrusted;
  readonly trustPullRequest = trustPullRequest;
  readonly excludePath = excludePath;
  readonly listDirectory = listDirectory;
}

export type Changeset = ForwardedChangeset;

export const changeset = new ForwardedChangeset();
