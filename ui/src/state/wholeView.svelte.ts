// The changeset as a whole, held once by the store: Problems, the queue panel and a file tab on the
// whole changeset read it while the review shows a range or a commit, whose own layout is not the
// changeset's. The store reads it when the changeset is on screen and feeds it here, and reads it
// itself when it is not; a mark is written into it from the review's own announcement, so only a
// settle, a target change and a resync read it again.

import type { BaseDiff } from "../generated/BaseDiff";
import type { CommentAt } from "../generated/CommentAt";
import type { NonTextKind } from "../generated/NonTextKind";
import { STALE_NOTICE } from "../lib/notice";
import { notices } from "./notices.svelte";
import { Latest } from "./resource";
import { newReviewCells } from "./reviewCells.svelte";
import { type Answer, reviewMarks } from "./reviewMarks";
import {
  type FileLayout,
  type FileState,
  type Parts,
  type ReviewedFile,
  type ViewLayout,
  partsOf,
} from "./reviewModel";
import type { WriteListener } from "./reviewSource";
import { session } from "./session.svelte";
import { type Shown, isCurrent } from "./shown";
import type { ViewSource } from "./viewScope.svelte";
import { CHANGESET } from "./view";

class Whole implements ViewSource {
  /** The changeset's files and base; none before the first read. */
  layout = $state.raw<ViewLayout>();
  private cells = newReviewCells();
  /** The files of `layout` by path, set with it. */
  private byPath = new Map<string, FileLayout>();
  /** The newest change of the backend this view reflects. */
  private seen = -1;
  private reads = new Latest();
  /** Whether the changeset this view belongs to was switched away from. */
  private ended: () => boolean;
  private marks = reviewMarks({
    backend: () => session.connected(),
    admit: (shown) => this.admit(shown),
    current: () => this.layout?.shown,
    apply: (update) => this.applyUpdate(update),
    refresh: () => this.reload(),
    hunks: (path) => this.cells.state(path)?.review.hunks ?? [],
    ended: () => this.ended(),
  });

  constructor(ended: () => boolean) {
    this.ended = ended;
  }

  /** The revision the whole changeset was read at; none before the first read. */
  get revision(): number | undefined {
    return this.layout?.shown.revision;
  }

  /** Reads the changeset as a whole again. Only the latest read counts. */
  async reload(): Promise<void> {
    const isLatest = this.reads.begin();
    try {
      const parts = partsOf(await session.connected().manifest(CHANGESET), undefined);
      if (isLatest()) this.adopt(parts);
    } catch (error) {
      if (isLatest()) notices.report(error);
    }
  }

  /** Shows what the store read of the changeset as a whole, when it is the view on screen. */
  adopt(parts: Parts) {
    const newer = this.cells.order.load(
      parts.seq,
      parts.layout.paths,
      isCurrent(parts.layout.shown, this.layout?.shown),
    );
    this.byPath = new Map(parts.layout.files.map((file) => [file.path, file]));
    this.layout = parts.layout;
    this.seen = Math.max(this.seen, parts.seq);
    this.cells.replace(parts.states, newer);
  }

  /**
   * The backend announced a mark of the changeset's file, which a range or a commit on screen does
   * not show: it is written into the whole changeset here.
   */
  heard(update: Answer, revision: number) {
    if (revision !== this.layout?.shown.revision || update.seq <= this.seen) return;
    this.applyUpdate(update);
  }

  /** Drops what the whole changeset held. */
  clear() {
    this.reads.cancel();
    this.layout = undefined;
    this.byPath = new Map();
    this.seen = -1;
    this.cells.reset();
  }

  /** A file of the whole changeset with its review state as it is now; to be read, not kept. */
  reviewed(path: string): ReviewedFile | undefined {
    const file = this.layout ? this.byPath.get(path) : undefined;
    const state = this.cells.file(path);
    return file && state && { ...file, ...state };
  }

  // The `ViewSource` the store hands out for a file tab on the whole changeset.
  currentShown(): Shown | undefined {
    return this.layout?.shown;
  }

  currentLayout(): ViewLayout | undefined {
    return this.layout;
  }

  layoutOf(path: string): FileLayout | undefined {
    return this.layout ? this.byPath.get(path) : undefined;
  }

  state(path: string): FileState | undefined {
    return this.cells.state(path);
  }

  version(path: string): number {
    return this.cells.version(path);
  }

  subscribe(listener: WriteListener): () => void {
    return this.cells.subscribe(listener);
  }

  admit(shown: Shown): boolean {
    if (this.layout && isCurrent(shown, this.layout.shown)) return true;
    notices.show(STALE_NOTICE);
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

  baseDiff(shown: Shown, path: string, hunk: number): Promise<BaseDiff | undefined> {
    return this.marks.baseDiff(shown, path, hunk);
  }

  commentTarget(shown: Shown, path: string, at: CommentAt) {
    return this.marks.commentTarget(shown, path, at);
  }

  private applyUpdate(update: Answer) {
    this.seen = Math.max(this.seen, update.seq);
    if (this.cells.has(update.path) && this.cells.order.file(update.path, update.seq)) {
      this.cells.put(update.path, { review: update.review, completion: update.completion });
    }
  }
}

export type WholeView = Whole;

export const newWhole = (ended: () => boolean): WholeView => new Whole(ended);
