/**
 * Tells the backend whether the window shows the review screen, once per change. The backend starts
 * the language servers of the changeset on view for it, so showing it is told when the changeset the
 * window asked for is the one on view, and leaving it at once. A backend starts out with no screen shown.
 */
export class ScreenReport {
  private told?: boolean;
  private readonly send: (shown: boolean) => void;

  constructor(send: (shown: boolean) => void) {
    this.send = send;
  }

  /** The backend does not know what the window showed: a new connection, or another project. */
  forget() {
    this.told = undefined;
  }

  update(shown: boolean, onView: boolean) {
    if (shown && !onView) return;
    const was = this.told ?? false;
    this.told = shown;
    if (shown !== was) this.send(shown);
  }
}
