// Whether an editor's caret still decides which stop is current.

/**
 * Navigation and presses choose the current stop without moving the carets of the editors that are
 * not shown it (the base editor of a split view, a caret left in another hunk), so a caret decides
 * the stop only once it has moved since the choice.
 */
export class CaretStop<V> {
  private at = new Map<V, number>();

  /** A stop was chosen while the carets of the editors were at `heads`. */
  chosen(heads: Iterable<[V, number]>) {
    this.at = new Map(heads);
  }

  /** The caret of `view`, at `head`, moved after the stop was chosen. */
  moved(view: V, head: number): boolean {
    return this.at.get(view) !== head;
  }
}
