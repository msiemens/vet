// A view of the changeset that file tabs read while it is not the one the review screen shows: the
// changeset as a whole, or one of its commits. A tab opened at a commit stays at it, and a tab opened
// on the whole changeset stays on that when a commit is selected, so each reads its own manifest and
// holds its own review state, which the store of the view on screen cannot give it.

import { SvelteMap } from "svelte/reactivity";
import type { BaseDiff } from "../generated/BaseDiff";
import type { CommentAt } from "../generated/CommentAt";
import type { CommitUnit } from "../generated/CommitUnit";
import type { NonTextKind } from "../generated/NonTextKind";
import type { ReviewUpdate } from "../generated/ReviewUpdate";
import { errorText } from "../lib/errorText";
import type { Backend } from "./backend";
import { type FileLayout, type FileState, type Parts, type ViewLayout, partsOf } from "./reviewModel";
import { ReviewOrder } from "./reviewOrder";
import type { ReviewSource, WriteListener } from "./reviewSource";
import { LatestLoad } from "./latestLoad";
import { session } from "./session.svelte";
import { STALE_NOTICE, type Shown, isCurrent } from "./shown";
import { CHANGESET, commitView } from "./view";

/** What a file tab reads and does in its view: the changeset store implements it for the view on screen, `ViewScope` for the others. */
export interface ViewSource extends ReviewSource {
  /** What the layout is of; none before the first manifest. Reads nothing reactively. */
  currentShown(): Shown | undefined;
  currentLayout(): ViewLayout | undefined;
  /** The structure of a file of the view. */
  layoutOf(path: string): FileLayout | undefined;
  toggleHunk(shown: Shown, path: string, hunk: number): Promise<void>;
  toggleFlag(shown: Shown, path: string, hunk: number): Promise<void>;
  toggleItem(shown: Shown, path: string, kind: NonTextKind): Promise<void>;
  toggleFileSeen(shown: Shown, path: string): Promise<void>;
  reconfirmFile(shown: Shown, path: string): Promise<void>;
  baseDiff(shown: Shown, path: string, hunk: number): Promise<BaseDiff | undefined>;
  commentTarget(shown: Shown, path: string, at: CommentAt): ReturnType<Backend["commentTarget"]>;
}

/** The changeset as a whole, or the commit with the full id. */
export type ScopeSpec = { type: "changeset" } | { type: "commit"; id: string };

export const scopeKey = (spec: ScopeSpec): string => (spec.type === "changeset" ? "changeset" : spec.id);

interface Loaded {
  parts: Parts;
  commit?: CommitUnit;
}

class Scope implements ViewSource {
  /** The view's files; none before the first manifest. */
  layout = $state.raw<ViewLayout>();
  /** The commit of a commit's scope, once the commits are read. */
  commit = $state.raw<CommitUnit>();
  /** The commit is not among the changeset's any more. */
  gone = $state(false);
  private cells = new SvelteMap<string, FileState>();
  private layouts = $derived(new Map(this.layout?.files.map((file) => [file.path, file])));
  private listeners = new Set<WriteListener>();
  private versions = new Map<string, number>();
  private clock = 0;
  private order = new ReviewOrder();
  private loads = new LatestLoad<Loaded | "gone">({
    busy: () => undefined,
    apply: (loaded) => this.apply(loaded),
  });

  constructor(
    private backend: Backend,
    readonly spec: ScopeSpec,
    private marked: () => void,
  ) {}

  private pending: Promise<void> = Promise.resolve();

  /** Reads the manifest of the view; the latest read counts. */
  load(): Promise<void> {
    this.pending = this.loads
      .load(() => this.fetch())
      .then(
        () => undefined,
        (error: unknown) => {
          session.notice = errorText(error);
        },
      );
    return this.pending;
  }

  /** Settles when the read underway, if there is one, is over. */
  whenLoaded(): Promise<void> {
    return this.pending;
  }

  private async fetch(): Promise<Loaded | "gone"> {
    if (this.spec.type === "changeset") return { parts: partsOf(await this.backend.manifest(CHANGESET), undefined) };
    const id = this.spec.id;
    const commits = await this.backend.commits();
    const index = commits.findIndex((c) => c.commit.id === id);
    if (index < 0) return "gone";
    return { parts: partsOf(await this.backend.manifest(commitView(index)), index), commit: commits[index] };
  }

  private apply(loaded: Loaded | "gone") {
    if (loaded === "gone") {
      this.gone = true;
      this.write(new Map(), "manifest");
      return;
    }
    const { parts, commit } = loaded;
    const newer = this.order.load(parts.seq, parts.layout.paths, isCurrent(parts.layout.shown, this.layout?.shown));
    const kept = new Map(newer.files.size ? this.cells : []);
    this.gone = false;
    this.commit = commit;
    this.layout = parts.layout;
    this.write(new Map([...parts.states].map(([path, state]) => [path, kept.get(path) ?? state])), "manifest");
  }

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

  subscribe(listener: WriteListener): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  state(path: string): FileState | undefined {
    return this.cells.get(path);
  }

  version(path: string): number {
    return this.versions.get(path) ?? 0;
  }

  layoutOf(path: string): FileLayout | undefined {
    return this.layouts.get(path);
  }

  currentLayout(): ViewLayout | undefined {
    return this.layout;
  }

  currentShown(): Shown | undefined {
    return this.layout?.shown;
  }

  private admit(shown: Shown): boolean {
    if (this.layout && isCurrent(shown, this.layout.shown)) return true;
    session.notice = STALE_NOTICE;
    return false;
  }

  /** Sends a request that marks and takes its answer into the state, unless the view was replaced meanwhile. */
  private async review(sent: Shown, request: Promise<ReviewUpdate | ReviewUpdate[]>) {
    try {
      const updates = [await request].flat();
      if (this.layout && isCurrent(sent, this.layout.shown)) for (const update of updates) this.applyReview(update);
      this.marked();
    } catch (error) {
      await this.load();
      this.reportFailure(error, sent.revision);
    }
  }

  private applyReview(update: ReviewUpdate) {
    if (!this.cells.has(update.path) || !this.order.file(update.path, update.seq)) return;
    this.write(new Map([[update.path, { review: update.review, completion: update.completion }]]), "update");
  }

  private reportFailure(error: unknown, sent: number) {
    const current = this.layout?.shown.revision;
    session.notice = current !== undefined && current !== sent ? STALE_NOTICE : `Request failed: ${errorText(error)}`;
  }

  async toggleHunk(shown: Shown, path: string, hunk: number) {
    if (this.admit(shown)) await this.review(shown, this.backend.toggleHunk(shown.revision, path, hunk, shown.view));
  }

  async toggleFlag(shown: Shown, path: string, hunk: number) {
    if (this.admit(shown)) await this.review(shown, this.backend.toggleFlag(shown.revision, path, hunk, shown.view));
  }

  async toggleItem(shown: Shown, path: string, kind: NonTextKind) {
    if (this.admit(shown)) await this.review(shown, this.backend.toggleItem(shown.revision, path, kind, shown.view));
  }

  async toggleFileSeen(shown: Shown, path: string) {
    if (this.admit(shown)) await this.review(shown, this.backend.toggleFileSeen(shown.revision, path, shown.view));
  }

  async reconfirmFile(shown: Shown, path: string) {
    if (!this.admit(shown)) return;
    if (shown.view.type === "changeset") {
      await this.review(shown, this.backend.reconfirmFile(shown.revision, path));
      return;
    }
    const hunks = this.cells.get(path)?.review.hunks ?? [];
    for (const [hunk, state] of hunks.entries()) {
      if (state === "seenAgainstOldBase") await this.toggleHunk(shown, path, hunk);
    }
  }

  async baseDiff(shown: Shown, path: string, hunk: number): Promise<BaseDiff | undefined> {
    if (!this.admit(shown)) return undefined;
    try {
      return await this.backend.baseDiff(shown.revision, path, hunk);
    } catch (error) {
      this.reportFailure(error, shown.revision);
      return undefined;
    }
  }

  async commentTarget(shown: Shown, path: string, at: CommentAt) {
    if (!this.admit(shown)) throw new Error(STALE_NOTICE);
    return this.backend.commentTarget(shown.revision, path, at, shown.view);
  }
}

export type ViewScope = Scope;

/** A scope of `spec`; `marked` is called after a mark made in it, since other views may show the same lines. */
export const newScope = (backend: Backend, spec: ScopeSpec, marked: () => void): ViewScope =>
  new Scope(backend, spec, marked);
