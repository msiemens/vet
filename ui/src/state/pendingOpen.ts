/**
 * A changeset this window asked the backend to open, from the request until the backend has
 * announced it (`changesetOpened`, which precedes the answer) or answered without: it failed or the
 * changeset was on view already. Until then the changeset on view is not what was asked for.
 */
export class PendingOpen {
  private seq = 0;
  private active = false;
  /** The changeset the latest request asked for. */
  private wanted = "";
  private busy: (on: boolean) => void;

  constructor(busy: (on: boolean) => void) {
    this.busy = busy;
  }

  /** A request for the changeset `id` is sent; the token goes to `settle` when it is answered. */
  begin(id: string): number {
    this.active = true;
    this.wanted = id;
    this.busy(true);
    return ++this.seq;
  }

  /**
   * The backend announced the changeset `id`. Whether the views are to fetch it: not while a newer
   * request is pending, whose own announcement follows (an earlier request's would show its changeset
   * for a moment).
   */
  announced(id: string): boolean {
    if (!this.active) return true;
    if (id !== this.wanted) return false;
    this.active = false;
    this.busy(false);
    return true;
  }

  /**
   * The request `token` was answered. Whether nothing was announced for it, so the changeset on view
   * is still the backend's and is to be fetched again. A newer request still pending stays pending.
   */
  settle(token: number): boolean {
    if (token !== this.seq || !this.active) return false;
    this.active = false;
    this.busy(false);
    return true;
  }
}
