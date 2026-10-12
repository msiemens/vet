// A file's section in a `ChangesetView`: what the view, its height model and the thread layer
// all read and write about it.

import type { BaseDiff } from "../../../generated/BaseDiff";
import type { FileLayout } from "../../../state/reviewModel";
import type { Card } from "./changeCard";
import type { FileProgress } from "./fileHeader";
import type { Handle, Hooks, Loaded } from "./mount";
import type { WrappedSides } from "./sectionLayout";
import type { CardData, InlineCard, Placement } from "./threadPlacement";

/** The part of a section the thread layer keeps. */
export interface ThreadState {
  placement: Placement;
  /** Cards below lines: the placed threads and the form for a new one. */
  inline: InlineCard[];
  /** Cards of the file's own threads, between the change cards and the first hunk. */
  blockEl: HTMLElement;
  blockCards: CardData[];
  /** What `blockEl` was built from. */
  blockSignature: string;
  /** Height of the block, which `--vet-threads` reports to the body's layout. */
  block: number;
  /** The height each card adds to the section's height, by `cardKey`. */
  counted: Map<string, number>;
}

export interface Section extends ThreadState, FileProgress {
  file: FileLayout;
  /** The version of the file's review state that the section last drew; the state itself is the source's. */
  drawn: number;
  el: HTMLElement;
  header: HTMLElement;
  body: HTMLElement;
  /** One per non-text item of the file, above the first hunk. */
  cards: Card[];
  top: number;
  height: number;
  /** Offset of each card from the top of the section. */
  cardTops: number[];
  cardsHeight: number;
  /** Estimated offset of each hunk bar from the top of the section. */
  hunkTops: number[];
  hooks: Hooks;
  /** The base diffs open under hunk bars, by hunk: what the hunk was seen against, as it was. */
  baseDiffs: Map<number, BaseDiff>;
  /** The editor wraps long lines. */
  wrap: boolean;
  /** The wrapped lines of each side's text: known once the text has arrived, and only while `wrap` holds. */
  wrapped: WrappedSides;
  loaded?: Loaded;
  loading?: Promise<Loaded>;
  handle?: Handle;
}
