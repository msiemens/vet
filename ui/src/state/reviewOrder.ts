// Review state reaches the window by several routes (answers, events, manifests) that do not arrive in
// the order the backend produced them. Every one carries the project's change counter as of the state
// it was read from; this keeps the counter of what the window holds and says what a newer arrival may
// replace, so that the last to arrive is not taken for the latest.

/** What a manifest does not replace because the window holds a newer state of it. */
export interface Newer {
  /** The files whose held state is newer than the manifest's. */
  readonly files: ReadonlySet<string>;
  /** The held completion of the view. */
  readonly completion: boolean;
  /** The held completion of the changeset as a whole. */
  readonly changeset: boolean;
}

const NOTHING: Newer = { files: new Set(), completion: false, changeset: false };

export class ReviewOrder {
  private files = new Map<string, number>();
  private view = -1;
  private whole = -1;

  /**
   * A manifest read at `seq` of `paths` is being applied. With `keeping`, it is of what is held (the
   * same revision and view) and does not replace what is newer; otherwise it replaces everything.
   */
  load(seq: number, paths: readonly string[], keeping: boolean): Newer {
    if (!keeping) {
      this.files = new Map(paths.map((path) => [path, seq]));
      this.view = this.whole = seq;
      return NOTHING;
    }
    const heldAt = (path: string) => this.files.get(path) ?? -1;
    const files = new Set(paths.filter((path) => heldAt(path) > seq));
    this.files = new Map(paths.map((path) => [path, Math.max(heldAt(path), seq)]));
    const completion = this.view > seq;
    const changeset = this.whole > seq;
    this.view = Math.max(this.view, seq);
    this.whole = Math.max(this.whole, seq);
    return { files, completion, changeset };
  }

  /** Whether an update of `path` read at `seq` is newer than the state held; if so, it is held now. */
  file(path: string, seq: number): boolean {
    if (seq <= (this.files.get(path) ?? -1)) return false;
    this.files.set(path, seq);
    return true;
  }

  /** Whether a completion of the view read at `seq` is newer than the one held; if so, it is held now. */
  completion(seq: number): boolean {
    if (seq <= this.view) return false;
    this.view = seq;
    return true;
  }

  /** Whether a completion of the changeset read at `seq` is newer than the one held; if so, it is held now. */
  changeset(seq: number): boolean {
    if (seq <= this.whole) return false;
    this.whole = seq;
    return true;
  }
}
