// The changeset as a whole, held once by the store: Problems, the queue panel and a file tab on the
// whole changeset read it while the review shows a range or a commit, whose own layout is not the
// changeset's. The store reads it when the changeset is on screen and feeds it here, and reads it
// itself when it is not; a mark is written into it from the review's own announcement, so only a
// settle, a target change and a resync read it again.

import type { Backend } from "./backend";
import { HeldView } from "./heldView.svelte";
import { notices } from "./notices.svelte";
import { Latest } from "./resource";
import type { Answer } from "./reviewMarks";
import { type Parts, partsOf } from "./reviewModel";
import { session } from "./session.svelte";
import type { ViewSource } from "./viewScope.svelte";
import { CHANGESET } from "./view";

class Whole extends HeldView implements ViewSource {
  private reads = new Latest();
  /** Whether the changeset this view belongs to was switched away from. */
  private isEnded: () => boolean;

  constructor(ended: () => boolean) {
    super();
    this.isEnded = ended;
  }

  protected override client(): Backend {
    return session.connected();
  }

  protected override refresh(): Promise<unknown> {
    return this.reload();
  }

  protected override ended(): boolean {
    return this.isEnded();
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
    this.take(parts);
  }

  /**
   * The backend announced a mark of the changeset's file, which a range or a commit on screen does
   * not show: it is written into the whole changeset here.
   */
  heard(update: Answer, revision: number) {
    if (revision !== this.layout?.shown.revision || update.seq <= this.seen) return;
    this.putAnswer(update);
  }

  /** Drops what the whole changeset held. */
  clear() {
    this.reads.cancel();
    this.clearHeld();
  }
}

export type WholeView = Whole;

export const newWhole = (ended: () => boolean): WholeView => new Whole(ended);
