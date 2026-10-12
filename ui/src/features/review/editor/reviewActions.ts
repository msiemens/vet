// Seen marks of a `ChangesetView`: the requests that flip them and the rendering of the state that
// comes back. The backend decides what a toggle does (the direction, what a file toggle covers);
// the view only asks and draws what it is told.

import type { BaseDiff } from "../../../generated/BaseDiff";
import type { NonTextKind } from "../../../generated/NonTextKind";
import type { ReviewUpdate } from "../../../generated/ReviewUpdate";
import { renderProgress } from "./fileHeader";
import { setReview } from "./reviewState";
import type { Section } from "./section";

export interface ReviewRequests {
  /**
   * Asks the owner of the review state to flip a hunk's seen state at `revision`; the backend
   * decides the direction. Its new state comes back through `renderReview`.
   */
  toggleHunk(revision: number, path: string, hunk: number): Promise<void>;
  /** The same for a hunk's flag for later review. */
  toggleFlag(revision: number, path: string, hunk: number): Promise<void>;
  /** The same for the acknowledgement of a non-text item. */
  toggleItem(revision: number, path: string, kind: NonTextKind): Promise<void>;
  /** The same for everything in a file. */
  toggleFileSeen(revision: number, path: string): Promise<void>;
  /** Marks the file's hunks seen against an old base seen again, against the base now. */
  reconfirmFile(revision: number, path: string): Promise<void>;
  /**
   * What a hunk seen against an old base looked like on the base side when it was marked; none if it
   * could not be fetched. Absent where the view has no such diff to show (a range of versions).
   */
  baseDiff?(revision: number, path: string, hunk: number): Promise<BaseDiff | undefined>;
}

export class ReviewActions {
  constructor(
    private revision: number,
    private requests: ReviewRequests,
  ) {}

  /** Asks the backend to mark everything in the file (hunks and change cards) seen, or unseen if it all already is. */
  file({ file }: Section): Promise<void> {
    return this.requests.toggleFileSeen(this.revision, file.path);
  }

  /** Asks the backend to mark the file's hunks seen against an old base seen again. */
  reconfirm({ file }: Section): Promise<void> {
    return this.requests.reconfirmFile(this.revision, file.path);
  }

  /** Asks for the base diff of the file's `hi`-th hunk. */
  baseDiff({ file }: Section, hi: number): Promise<BaseDiff | undefined> {
    return this.requests.baseDiff?.(this.revision, file.path, hi) ?? Promise.resolve(undefined);
  }

  /** Asks the backend to flip the file's `hi`-th hunk. */
  hunk({ file }: Section, hi: number): Promise<void> {
    return this.requests.toggleHunk(this.revision, file.path, hi);
  }

  /** Asks the backend to flip the flag of the file's `hi`-th hunk. */
  flag({ file }: Section, hi: number): Promise<void> {
    return this.requests.toggleFlag(this.revision, file.path, hi);
  }

  /** Asks the backend to flip the acknowledgement of the file's `ci`-th change card. */
  card({ file }: Section, ci: number): Promise<void> {
    return this.requests.toggleItem(this.revision, file.path, file.nonText[ci].kind);
  }
}

/** Renders a file's review state and progress as the backend sent them. */
export function renderReview(s: Section, { review, completion }: ReviewUpdate) {
  s.review = review;
  s.completion = completion;
  s.handle?.dispatchAll(setReview.of(review));
  s.cards.forEach((card, i) => card.setAcknowledged(review.acknowledged.includes(s.file.nonText[i].kind)));
  renderProgress(s, review, completion);
}
