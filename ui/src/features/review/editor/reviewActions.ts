// Seen marks of a `ChangesetView`: the requests that flip them and the rendering of the state that
// comes back. The backend decides what a toggle does (the direction, what a file toggle covers);
// the view only asks and draws what it is told.

import type { BaseDiff } from "../../../generated/BaseDiff";
import type { NonTextKind } from "../../../generated/NonTextKind";
import type { FileState } from "../../../state/reviewModel";
import type { Shown } from "../../../state/shown";
import { renderProgress } from "./fileHeader";
import { setReview } from "./reviewState";
import type { Section } from "./section";

export interface ReviewRequests {
  /**
   * Asks the owner of the review state to flip a hunk's seen state in `shown`, what the view was built
   * for; the backend decides the direction. Its new state reaches the view through the review source.
   */
  toggleHunk(shown: Shown, path: string, hunk: number): Promise<void>;
  /** The same for a hunk's flag for later review. */
  toggleFlag(shown: Shown, path: string, hunk: number): Promise<void>;
  /** The same for the acknowledgement of a non-text item. */
  toggleItem(shown: Shown, path: string, kind: NonTextKind): Promise<void>;
  /** The same for everything in a file. */
  toggleFileSeen(shown: Shown, path: string): Promise<void>;
  /** Marks the file's hunks seen against an old base seen again, against the base now. */
  reconfirmFile(shown: Shown, path: string): Promise<void>;
  /**
   * What a hunk seen against an old base looked like on the base side when it was marked; none if it
   * could not be fetched. Absent where the view has no such diff to show (a range of versions).
   */
  baseDiff?(shown: Shown, path: string, hunk: number): Promise<BaseDiff | undefined>;
}

export class ReviewActions {
  constructor(
    private shown: Shown,
    private requests: ReviewRequests,
  ) {}

  /** Asks the backend to mark everything in the file (hunks and change cards) seen, or unseen if it all already is. */
  file({ file }: Section): Promise<void> {
    return this.requests.toggleFileSeen(this.shown, file.path);
  }

  /** Asks the backend to mark the file's hunks seen against an old base seen again. */
  reconfirm({ file }: Section): Promise<void> {
    return this.requests.reconfirmFile(this.shown, file.path);
  }

  /** Asks for the base diff of the file's `hi`-th hunk. */
  baseDiff({ file }: Section, hi: number): Promise<BaseDiff | undefined> {
    return this.requests.baseDiff?.(this.shown, file.path, hi) ?? Promise.resolve(undefined);
  }

  /** Asks the backend to flip the file's `hi`-th hunk. */
  hunk({ file }: Section, hi: number): Promise<void> {
    return this.requests.toggleHunk(this.shown, file.path, hi);
  }

  /** Asks the backend to flip the flag of the file's `hi`-th hunk. */
  flag({ file }: Section, hi: number): Promise<void> {
    return this.requests.toggleFlag(this.shown, file.path, hi);
  }

  /** Asks the backend to flip the acknowledgement of the file's `ci`-th change card. */
  card({ file }: Section, ci: number): Promise<void> {
    return this.requests.toggleItem(this.shown, file.path, file.nonText[ci].kind);
  }
}

/** Draws a file's review state and progress, as the source holds them at `version`. */
export function renderReview(s: Section, { review, completion }: FileState, version: number) {
  s.drawn = version;
  s.handle?.dispatchAll(setReview.of(review));
  s.cards.forEach((card, i) => card.setAcknowledged(review.acknowledged.includes(s.file.nonText[i].kind)));
  renderProgress(s, review, completion);
}
