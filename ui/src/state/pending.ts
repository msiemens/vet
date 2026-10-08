// The requests of a connection that wait for their answer, by id. A request given an
// `AbortSignal` tells the backend to cancel it when the signal fires, and rejects at once.

import { abortError } from "../lib/abort.ts";

export class Pending<T> {
  private waiting = new Map<number, { resolve: (value: T) => void; reject: (error: Error) => void }>();

  /**
   * The answer to request `id`. If `signal` fires first, `cancel(id)` runs and the promise rejects
   * with an abort; a signal that has fired already rejects without the request being waited for.
   */
  wait(id: number, signal: AbortSignal | undefined, cancel: (id: number) => void): Promise<T> {
    if (signal?.aborted) return Promise.reject(abortError());
    return new Promise((resolve, reject) => {
      const onAbort = () => {
        if (this.waiting.delete(id)) cancel(id);
        reject(abortError());
      };
      signal?.addEventListener("abort", onAbort, { once: true });
      const settled = () => signal?.removeEventListener("abort", onAbort);
      this.waiting.set(id, {
        resolve: (value) => {
          settled();
          resolve(value);
        },
        reject: (error) => {
          settled();
          reject(error);
        },
      });
    });
  }

  resolve(id: number, value: T) {
    this.waiting.get(id)?.resolve(value);
    this.waiting.delete(id);
  }

  reject(id: number, error: Error) {
    this.waiting.get(id)?.reject(error);
    this.waiting.delete(id);
  }

  rejectAll(error: Error) {
    for (const { reject } of this.waiting.values()) reject(error);
    this.waiting.clear();
  }
}
