// A view of the changeset that file tabs read while it is not the one the review screen shows: the
// changeset as a whole, or one of its commits. A tab opened at a commit stays at it, and a tab opened on
// the whole changeset stays on that when a commit is selected, so each reads its own manifest and holds
// its own review state, which the store of the view on screen cannot give it.

import type { BaseDiff } from "../generated/BaseDiff";
import type { CommentAt } from "../generated/CommentAt";
import type { CommitUnit } from "../generated/CommitUnit";
import type { NonTextKind } from "../generated/NonTextKind";
import { errorText } from "../lib/errorText";
import type { Backend } from "./backend";
import { LatestLoad } from "./latestLoad";
import { newReviewCells } from "./reviewCells.svelte";
import { type Answer, reviewMarks } from "./reviewMarks";
import { type FileLayout, type FileState, type Parts, type ViewLayout, partsOf } from "./reviewModel";
import type { ReviewSource, WriteListener } from "./reviewSource";
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
  /** Whether a request built for `shown` still is about this view; if not, the viewer is told so. */
  admit(shown: Shown): boolean;
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

/** How often a read is made again when the commits and the manifest turn out to be of different revisions. */
const PAIRING_ATTEMPTS = 3;

class Scope implements ViewSource {
  /** The view's files; none before the first manifest. */
  layout = $state.raw<ViewLayout>();
  /** The commit of a commit's scope, once the commits are read. */
  commit = $state.raw<CommitUnit>();
  /** The commit is not among the changeset's any more. */
  gone = $state(false);
  private cells = newReviewCells();
  /** The files of `layout` by path, set with it. */
  private byPath = new Map<string, FileLayout>();
  /** The newest change of the backend this scope's state reflects. */
  private seen = -1;
  /** A failure of reading was reported and nothing has been read since. */
  private reported = false;
  private pending: Promise<void> = Promise.resolve();
  private loads = new LatestLoad<Loaded | "gone">({
    busy: () => undefined,
    apply: (loaded) => this.apply(loaded),
  });
  private marks = reviewMarks({
    backend: () => this.backend,
    admit: (shown) => this.admit(shown),
    current: () => this.currentShown(),
    apply: (update, shown) => {
      this.applyUpdate(update);
      this.marked(shown, update);
    },
    refresh: () => this.load(),
    hunks: (path) => this.cells.state(path)?.review.hunks ?? [],
  });

  /** `marked` is told of every answer a mark got here, since the view on screen may show the same lines. */
  constructor(
    private backend: Backend,
    readonly spec: ScopeSpec,
    private marked: (shown: Shown, update: Answer) => void,
  ) {}

  /** Reads the manifest of the view; the latest read counts. */
  load(): Promise<void> {
    this.pending = this.loads
      .load(() => this.fetch())
      .then(
        () => {
          this.reported = false;
        },
        (error: unknown) => {
          if (!this.reported) session.notice = errorText(error);
          this.reported = true;
        },
      );
    return this.pending;
  }

  /** Settles when the read underway, if there is one, is over. */
  whenLoaded(): Promise<void> {
    return this.pending;
  }

  /**
   * The backend announced a mark of the changeset's file, as the answer to a request and as an event.
   * The changeset's own scope takes it as the store does; a commit's has no word of its lines in it,
   * so it reads again, unless it has what the update says already (its own mark).
   */
  heard(update: Answer, revision: number) {
    if (revision !== this.layout?.shown.revision || update.seq <= this.seen) return;
    if (this.spec.type === "changeset") this.applyUpdate(update);
    else void this.load();
  }

  /** The commit and the manifest it was read with are of one revision, else they are read again. */
  private async fetch(): Promise<Loaded | "gone"> {
    if (this.spec.type === "changeset") return { parts: partsOf(await this.backend.manifest(CHANGESET), undefined) };
    const id = this.spec.id;
    for (let attempt = 1; ; attempt++) {
      const { revision, commits } = await this.backend.commits();
      const index = commits.findIndex((c) => c.commit.id === id);
      if (index < 0) return "gone";
      const parts = partsOf(await this.backend.manifest(commitView(index)), index);
      if (parts.revision === revision) return { parts, commit: commits[index] };
      if (attempt >= PAIRING_ATTEMPTS) throw new Error("The commits changed while they were read");
    }
  }

  private apply(loaded: Loaded | "gone") {
    if (loaded === "gone") {
      this.gone = true;
      this.cells.clearedAll();
      return;
    }
    const { parts, commit } = loaded;
    const newer = this.cells.order.load(
      parts.seq,
      parts.layout.paths,
      isCurrent(parts.layout.shown, this.layout?.shown),
    );
    this.gone = false;
    this.commit = commit;
    this.byPath = new Map(parts.layout.files.map((file) => [file.path, file]));
    this.layout = parts.layout;
    this.seen = Math.max(this.seen, parts.seq);
    this.cells.replace(parts.states, newer);
  }

  private applyUpdate(update: Answer) {
    this.seen = Math.max(this.seen, update.seq);
    if (this.cells.has(update.path) && this.cells.order.file(update.path, update.seq)) {
      this.cells.put(update.path, { review: update.review, completion: update.completion });
    }
  }

  subscribe(listener: WriteListener): () => void {
    return this.cells.subscribe(listener);
  }

  state(path: string): FileState | undefined {
    return this.cells.state(path);
  }

  version(path: string): number {
    return this.cells.version(path);
  }

  layoutOf(path: string): FileLayout | undefined {
    // Reading the layout makes the lookup follow it.
    return this.layout ? this.byPath.get(path) : undefined;
  }

  currentLayout(): ViewLayout | undefined {
    return this.layout;
  }

  currentShown(): Shown | undefined {
    return this.layout?.shown;
  }

  admit(shown: Shown): boolean {
    if (this.layout && isCurrent(shown, this.layout.shown)) return true;
    session.notice = STALE_NOTICE;
    return false;
  }

  toggleHunk(shown: Shown, path: string, hunk: number) {
    return this.marks.toggleHunk(shown, path, hunk);
  }

  toggleFlag(shown: Shown, path: string, hunk: number) {
    return this.marks.toggleFlag(shown, path, hunk);
  }

  toggleItem(shown: Shown, path: string, kind: NonTextKind) {
    return this.marks.toggleItem(shown, path, kind);
  }

  toggleFileSeen(shown: Shown, path: string) {
    return this.marks.toggleFileSeen(shown, path);
  }

  reconfirmFile(shown: Shown, path: string) {
    return this.marks.reconfirmFile(shown, path);
  }

  baseDiff(shown: Shown, path: string, hunk: number) {
    return this.marks.baseDiff(shown, path, hunk);
  }

  commentTarget(shown: Shown, path: string, at: CommentAt) {
    return this.marks.commentTarget(shown, path, at);
  }
}

export type ViewScope = Scope;

/** A scope of `spec`; `marked` is told of every answer a mark got in it. */
export const newScope = (
  backend: Backend,
  spec: ScopeSpec,
  marked: (shown: Shown, update: Answer) => void,
): ViewScope => new Scope(backend, spec, marked);
