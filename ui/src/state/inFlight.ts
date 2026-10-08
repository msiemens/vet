import { abortError } from "../lib/abort.ts";

interface Underway<T> {
  request: Promise<T>;
  /** Aborts the request itself. */
  abort: AbortController;
  /** The callers that still want the answer. */
  waiting: number;
}

/** Requests that are being answered, so a caller asking the same question joins the one underway. */
export class InFlight<T> {
  private pending = new Map<string, Underway<T>>();

  /**
   * The answer to `key`: the request underway if there is one, else the result of `start`. A caller whose
   * `signal` aborts gives up at once (its promise rejects with an abort); the request itself is aborted
   * only when no caller wants it any more.
   */
  share(key: string, start: (signal: AbortSignal) => Promise<T>, signal?: AbortSignal): Promise<T> {
    if (signal?.aborted) return Promise.reject(abortError());
    let underway = this.pending.get(key);
    if (!underway) {
      const abort = new AbortController();
      const request: Promise<T> = start(abort.signal).finally(() => {
        if (this.pending.get(key) === created) this.pending.delete(key);
      });
      const created: Underway<T> = { request, abort, waiting: 0 };
      this.pending.set(key, created);
      underway = created;
    }
    const joined = underway;
    joined.waiting++;
    if (!signal) return joined.request;
    return new Promise<T>((resolve, reject) => {
      const leave = () => {
        signal.removeEventListener("abort", onAbort);
      };
      const onAbort = () => {
        leave();
        if (--joined.waiting === 0) {
          if (this.pending.get(key) === joined) this.pending.delete(key);
          joined.abort.abort();
        }
        reject(abortError());
      };
      signal.addEventListener("abort", onAbort, { once: true });
      joined.request.then(
        (value) => {
          leave();
          resolve(value);
        },
        (error: unknown) => {
          leave();
          reject(error instanceof Error ? error : new Error(String(error)));
        },
      );
    });
  }

  /** Forgets the requests underway; their callers still get their answers, later callers ask afresh. */
  clear() {
    this.pending.clear();
  }
}
