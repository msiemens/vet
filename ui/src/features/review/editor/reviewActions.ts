// Seen marks of a `ChangesetView`: the requests that flip them and the rendering of the state that
// comes back. The backend decides what a toggle does (the direction, what a file toggle covers);
// the view only asks and draws what it is told.

import type { NonTextKind } from "../../../generated/NonTextKind";
import type { ReviewUpdate } from "../../../generated/ReviewUpdate";
import { renderCount } from "./fileHeader";
import { setStates } from "./reviewState";
import type { Section } from "./section";

export interface ReviewRequests {
  /**
   * Asks the owner of the review state to flip a hunk's seen state at `revision`; the backend
   * decides the direction. Its new state comes back through `renderReview`.
   */
  toggleHunk(revision: number, path: string, hunk: number): Promise<void>;
  /** The same for the acknowledgement of a non-text item. */
  toggleItem(revision: number, path: string, kind: NonTextKind): Promise<void>;
  /** The same for everything in a file. */
  toggleFileSeen(revision: number, path: string): Promise<void>;
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

  /** Asks the backend to flip the file's `hi`-th hunk. */
  hunk({ file }: Section, hi: number): Promise<void> {
    return this.requests.toggleHunk(this.revision, file.path, hi);
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
  s.handle?.dispatchAll(setStates.of(review.hunks));
  s.cards.forEach((card, i) => card.setAcknowledged(review.acknowledged.includes(s.file.nonText[i].kind)));
  renderCount(s, completion);
}
