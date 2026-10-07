// A file's section in a `ChangesetView`: what the view, its height model and the thread layer
// all read and write about it.

import type { Completion } from "../../../generated/Completion";
import type { FileChange } from "../../../generated/FileChange";
import type { FileReview } from "../../../generated/FileReview";
import type { Card } from "./changeCard";
import type { Handle, Hooks, Loaded } from "./mount";
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

export interface Section extends ThreadState {
  file: FileChange;
  review: FileReview;
  completion: Completion;
  el: HTMLElement;
  header: HTMLElement;
  body: HTMLElement;
  count: HTMLElement;
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
  loaded?: Loaded;
  loading?: Promise<Loaded>;
  handle?: Handle;
}
