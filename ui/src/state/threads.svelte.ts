// Comment threads as the backend resolved them. Where a thread's anchor is now (exact,
// moved, detached) is the backend's; this holds and distributes it (SPEC I5).

import type { Thread } from "../generated/Thread";
import type { ThreadStatus } from "../generated/ThreadStatus";
import type { ThreadTarget } from "../generated/ThreadTarget";
import type { Backend } from "./backend";
import { changeset } from "./changeset.svelte";

class ThreadStore {
  /** In creation order, resolved at `revision`. Immutable snapshots, replaced as a whole. */
  items = $state.raw<Thread[]>([]);
  /** The manifest revision the anchors in `items` belong to. */
  revision = $state<number>();
  private backend?: Backend;
  /** Only the latest reload counts. */
  private seq = 0;
  private listeners = new Set<() => void>();

  /** Loads the threads and keeps them current; the returned function stops that. */
  start(backend: Backend): () => void {
    this.backend = backend;
    void this.reload();
    const off = backend.onEvent((event) => {
      if (event.session !== backend.session) return;
      // A settled snapshot invalidates the resolution; another connection changed a thread.
      if (event.type === "threadsChanged" || event.type === "snapshotSettled") void this.reload();
    });
    return () => {
      off();
      this.backend = undefined;
      this.set([], undefined);
    };
  }

  async reload() {
    const backend = this.backend;
    if (!backend) return;
    const seq = ++this.seq;
    try {
      const { revision, threads } = await backend.threads();
      if (seq === this.seq) this.set(threads, revision);
    } catch (error) {
      if (seq === this.seq)
        changeset.notice = `Could not load comments: ${error instanceof Error ? error.message : String(error)}`;
    }
  }

  /** Opens a thread; resolves whether it went through. */
  async create(revision: number, target: ThreadTarget, body: string): Promise<boolean> {
    return this.change(this.backend?.createThread(revision, target, body));
  }

  async reply(thread: number, body: string): Promise<boolean> {
    return this.change(this.backend?.reply(thread, body));
  }

  async setStatus(thread: number, status: ThreadStatus): Promise<boolean> {
    return this.change(this.backend?.setThreadStatus(thread, status));
  }

  /** Also how a detached thread is discarded. */
  async remove(thread: number): Promise<boolean> {
    try {
      await this.backend?.deleteThread(thread);
    } catch (error) {
      this.fail(error);
      return false;
    }
    this.set(
      this.items.filter((t) => t.id !== thread),
      this.revision,
    );
    return true;
  }

  /** Re-anchors a moved thread where it is now. */
  async confirm(thread: number): Promise<boolean> {
    return this.revision === undefined ? false : this.change(this.backend?.confirmAnchor(this.revision, thread));
  }

  /** Re-anchors an ambiguous thread at one of its candidates. */
  async attach(thread: number, candidate: number): Promise<boolean> {
    return this.revision === undefined
      ? false
      : this.change(this.backend?.attachAnchor(this.revision, thread, candidate));
  }

  /** Called after every change of `items`. */
  onChange(listener: () => void): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  /** Puts the thread a request returned into the list. */
  private async change(request?: Promise<Thread>): Promise<boolean> {
    if (!request) return false;
    try {
      const thread = await request;
      const known = this.items.some((t) => t.id === thread.id);
      this.set(
        known ? this.items.map((t) => (t.id === thread.id ? thread : t)) : [...this.items, thread],
        this.revision,
      );
      return true;
    } catch (error) {
      this.fail(error);
      return false;
    }
  }

  private set(items: Thread[], revision: number | undefined) {
    this.items = items;
    this.revision = revision;
    for (const listener of this.listeners) listener();
  }

  /** A rejected request is normally a stale revision: the threads are fetched again and the viewer repeats the action. */
  private fail(error: unknown) {
    changeset.notice = `Request failed: ${error instanceof Error ? error.message : String(error)}`;
    void this.reload();
  }
}

export const threads = new ThreadStore();
