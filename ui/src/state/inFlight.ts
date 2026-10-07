/** Requests that are being answered, so a caller asking the same question joins the one underway. */
export class InFlight<T> {
  private pending = new Map<string, Promise<T>>();

  /** The answer to `key`: the request underway if there is one, else the result of `start`. */
  share(key: string, start: () => Promise<T>): Promise<T> {
    const underway = this.pending.get(key);
    if (underway) return underway;
    const request = start().finally(() => {
      if (this.pending.get(key) === request) this.pending.delete(key);
    });
    this.pending.set(key, request);
    return request;
  }

  /** Forgets the requests underway; their callers still get their answers, later callers ask afresh. */
  clear() {
    this.pending.clear();
  }
}
