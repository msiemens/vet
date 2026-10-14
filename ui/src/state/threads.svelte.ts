// Comment threads as the backend resolved them. Where a thread's anchor is now (exact,
// moved, detached) is the backend's; this holds and distributes it (SPEC I5).

import type { EditChoice } from "../generated/EditChoice";
import type { Thread } from "../generated/Thread";
import type { ThreadStatus } from "../generated/ThreadStatus";
import type { ThreadTarget } from "../generated/ThreadTarget";
import type { Backend } from "./backend";
import { changeset } from "./changeset.svelte";
import { followProject, whileOpen } from "./followProject";
import { notices } from "./notices.svelte";
import { Resource } from "./resource";
import type { Shown } from "./shown";
import { rangeKey } from "./timelineRange";
import { CHANGESET } from "./view";

class ThreadStore {
  /** In creation order, resolved at `revision`. Immutable snapshots, replaced as a whole. */
  items = $state.raw<Thread[]>([]);
  /** The manifest revision the anchors in `items` belong to. */
  revision = $state<number>();
  /** The range the threads were read in (see `rangeKey`): an interdiff shows the new side's threads only. */
  key = "";
  private backend?: Backend;
  private reads = new Resource<{ revision: number; threads: Thread[]; key: string }>({
    apply: ({ revision, threads, key }) => this.set(threads, revision, key),
    failed: (error) => notices.failed("Could not load comments", error),
  });
  private listeners = new Set<() => void>();

  constructor() {
    whileOpen((backend) => this.start(backend));
  }

  /** Loads the threads and keeps them current; the returned function stops that. */
  private start(backend: Backend): () => void {
    this.backend = backend;
    const stop = followProject(backend, {
      // A settled snapshot invalidates the resolution; another connection changed a thread.
      reloadOn: ["threadsChanged", "snapshotSettled"],
      reload: () => void this.reload(),
      cancel: () => this.reads.cancel(),
      // The threads are the previous changeset's from the request on, until the new ones are read.
      clear: () => this.set([], undefined, ""),
    });
    return () => {
      stop();
      this.reads.cancel();
      this.backend = undefined;
      this.set([], undefined);
    };
  }

  async reload() {
    const backend = this.backend;
    if (!backend) return;
    // A commit has no threads of its own: the comments are the changeset's, as the Comments tool window shows them.
    const { range } = changeset;
    const view = range ? changeset.view : CHANGESET;
    await this.reads.load(async () => {
      const { revision, threads } = await backend.threads(view);
      return { revision, threads, key: rangeKey(range) };
    });
  }

  /** Opens a thread; resolves whether it went through. `admit` is the view `shown` is of, which says whether it still is the one asked about; by default the view on screen. */
  async create(
    shown: Shown,
    target: ThreadTarget,
    body: string,
    admit: (shown: Shown) => boolean = (s) => changeset.admit(s),
  ): Promise<boolean> {
    if (!admit(shown)) return false;
    return this.change(this.backend?.createThread(shown.revision, target, body, shown.view), shown.revision);
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

  /** Edits one of the viewer's own comments. */
  async editMessage(message: number, body: string): Promise<boolean> {
    return this.change(this.backend?.editMessage(message, body));
  }

  /** Settles a comment that was edited here and on the platform. */
  async keepEdit(message: number, keep: EditChoice): Promise<boolean> {
    return this.change(this.backend?.keepEdit(message, keep));
  }

  /** Removes one of the viewer's own comments; the thread goes with its first. */
  async removeMessage(message: number): Promise<boolean> {
    try {
      await this.backend?.removeMessage(message);
    } catch (error) {
      this.fail(error);
      return false;
    }
    await this.reload();
    return true;
  }

  /** Re-anchors a moved thread where it is now. */
  async confirm(thread: number): Promise<boolean> {
    return this.revision === undefined
      ? false
      : this.change(this.backend?.confirmAnchor(this.revision, thread), this.revision);
  }

  /** Re-anchors an ambiguous thread at one of its candidates. */
  async attach(thread: number, candidate: number): Promise<boolean> {
    return this.revision === undefined
      ? false
      : this.change(this.backend?.attachAnchor(this.revision, thread, candidate), this.revision);
  }

  /** Called after every change of `items`. */
  onChange(listener: () => void): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  /** Puts the thread a request returned into the list. `sent` is the revision the request named, if it did. */
  private async change(request?: Promise<Thread>, sent?: number): Promise<boolean> {
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
      this.fail(error, sent);
      return false;
    }
  }

  private set(items: Thread[], revision: number | undefined, key = this.key) {
    this.items = items;
    this.revision = revision;
    this.key = key;
    for (const listener of this.listeners) listener();
  }

  /** A rejected request is normally a stale revision: the threads are fetched again and the viewer repeats the action. */
  private fail(error: unknown, sent?: number) {
    changeset.reportFailure(error, sent);
    void this.reload();
  }
}

export const threads = new ThreadStore();
