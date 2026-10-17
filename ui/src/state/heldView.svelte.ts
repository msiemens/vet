// What every holder of a view's files has in common: the layout and the review state of its files, the
// order in which answers apply, and the marks it sends. The changeset on screen, the changeset as a
// whole and a commit's scope each are one; they differ in how they read the view and what an answer
// to a mark does besides taking it in.

import { untrack } from "svelte";
import type { BaseDiff } from "../generated/BaseDiff";
import type { CommentAt } from "../generated/CommentAt";
import type { NonTextKind } from "../generated/NonTextKind";
import { STALE_NOTICE } from "../lib/notice";
import type { Backend } from "./backend";
import { notices } from "./notices.svelte";
import { type ReviewCells, newReviewCells } from "./reviewCells.svelte";
import { type Answer, reviewMarks } from "./reviewMarks";
import type { Newer } from "./reviewOrder";
import type { FileLayout, FileState, Parts, ReviewedFile, ViewLayout } from "./reviewModel";
import type { WriteListener } from "./reviewSource";
import { type Shown, isCurrent } from "./shown";

export abstract class HeldView {
  /** The view's files and base; none before the first read. */
  layout = $state.raw<ViewLayout>();
  protected readonly cells: ReviewCells = newReviewCells();
  /** The newest change of the backend this view reflects. */
  protected seen = -1;
  // eslint-disable-next-line svelte/prefer-svelte-reactivity -- rebuilt with the layout, never mutated
  private byPath = $derived(new Map(this.layout?.files.map((file) => [file.path, file])));
  private marks = reviewMarks({
    backend: () => this.client(),
    admit: (shown) => this.admit(shown),
    current: () => this.layout?.shown,
    apply: (update, shown) => this.applyAnswer(update, shown),
    refresh: () => this.refresh(),
    hunks: (path) => this.cells.state(path)?.review.hunks ?? [],
    ended: () => this.ended(),
  });

  protected abstract client(): Backend;

  /** Reads the view anew, after a mark was rejected. */
  protected abstract refresh(): Promise<unknown>;

  /** Whether the holder was switched away from: a failure of its marks is not told. */
  protected ended(): boolean {
    return false;
  }

  /** Takes the answer to a mark in `shown` into the review state. */
  // eslint-disable-next-line @typescript-eslint/no-unused-vars -- the signature subclasses override
  protected applyAnswer(update: Answer, shown: Shown) {
    this.putAnswer(update);
  }

  /** Whether a request built for `shown` still is about this view; if not, the viewer is told so. */
  admit(shown: Shown): boolean {
    if (this.layout && isCurrent(shown, this.layout.shown)) return true;
    notices.show(STALE_NOTICE);
    return false;
  }

  /** Holds what was read: the layout, and the states in the order the read and the answers give. */
  protected take(parts: Parts, alongside?: (newer: Newer) => void): Newer {
    const newer = this.cells.order.load(
      parts.seq,
      parts.layout.paths,
      isCurrent(parts.layout.shown, this.layout?.shown),
    );
    this.layout = parts.layout;
    this.seen = Math.max(this.seen, parts.seq);
    alongside?.(newer);
    this.cells.replace(parts.states, newer);
    return newer;
  }

  protected putAnswer(update: Answer) {
    this.seen = Math.max(this.seen, update.seq);
    if (this.cells.has(update.path) && this.cells.order.file(update.path, update.seq)) {
      this.cells.put(update.path, { review: update.review, completion: update.completion });
    }
  }

  /** Drops everything held. */
  protected clearHeld() {
    this.layout = undefined;
    this.seen = -1;
    this.cells.reset();
  }

  protected setFilesSeen(shown: Shown, paths: string[], seen: boolean) {
    return this.marks.setFilesSeen(shown, paths, seen);
  }

  /** A file with its review state as it is now; to be read, not kept. */
  reviewed(path: string): ReviewedFile | undefined {
    const file = this.layout ? this.byPath.get(path) : undefined;
    const state = this.cells.file(path);
    return file && state && { ...file, ...state };
  }

  currentShown(): Shown | undefined {
    return untrack(() => this.layout?.shown);
  }

  currentLayout(): ViewLayout | undefined {
    return untrack(() => this.layout);
  }

  layoutOf(path: string): FileLayout | undefined {
    // Reading the layout makes the lookup follow it.
    return this.layout ? this.byPath.get(path) : undefined;
  }

  file(path: string): FileState | undefined {
    return this.cells.file(path);
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
}
