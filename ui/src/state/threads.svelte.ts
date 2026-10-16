// A session's comment threads as the backend resolved them, one list per view: the changeset's and the
// range on view. Where a thread's anchor is now (exact, moved, detached) is the backend's; this holds and
// distributes it (SPEC I5).

import type { EditChoice } from "../generated/EditChoice";
import type { Thread } from "../generated/Thread";
import type { ThreadStatus } from "../generated/ThreadStatus";
import type { ThreadTarget } from "../generated/ThreadTarget";
import type { Backend } from "./backend";
import { notices } from "./notices.svelte";
import { Resource } from "./resource";
import { atRevision } from "./revision";
import type { Shown } from "./shown";
import { type ThreadCell, cellFor, cellListed, replaces } from "./threadCells";
import { type Range, rangeKey, rangeView } from "./timelineRange";
import { CHANGESET } from "./view";

/** What the threads of a session ask of it. */
export interface ThreadsHost {
  backend(): Backend;
  /** The session ended: what it started counts for nothing. */
  ended(): boolean;
  /** Whether `shown` is the view on screen. */
  admit(shown: Shown): boolean;
  reportFailure(error: unknown, sent?: number): void;
}

type Where = Pick<Range, "from" | "to">;
type Which = "whole" | "range";
const CELLS: readonly Which[] = ["whole", "range"];
const NONE: Thread[] = [];

class Threads {
  /** The whole changeset's threads; none before the first read. */
  whole = $state.raw<ThreadCell>();
  /** The threads of the range on view; held only while the review screen shows one. */
  range = $state.raw<ThreadCell>();
  /** The range the review view is of. */
  private ranged = $state.raw<Where>();
  private reviewShown = false;
  private host: ThreadsHost;
  private wholeReads = new Resource<ThreadCell>({
    apply: (cell) => this.take("whole", cell),
    failed: (error) => this.failed(error),
  });
  private rangeReads = new Resource<ThreadCell>({
    apply: (cell) => this.take("range", cell),
    failed: (error) => this.failed(error),
  });
  private listeners = new Set<() => void>();

  constructor(host: ThreadsHost) {
    this.host = host;
  }

  /** The threads beside the view on screen: its own once read, else the changeset's. */
  get items(): Thread[] {
    return this.listed?.items ?? NONE;
  }

  /** The threads of the changeset as a whole, whatever view is on screen. */
  get wholeItems(): Thread[] {
    return this.whole?.items ?? NONE;
  }

  /** The cell read for a view of `range`; none for the changeset or a commit means the changeset's. */
  of(range: Where | undefined): ThreadCell | undefined {
    return cellFor({ whole: this.whole, range: this.range }, range);
  }

  /** The review view is of `range` now (none for the changeset or a commit). */
  viewed(range: Where | undefined) {
    const moved = rangeKey(range) !== rangeKey(this.ranged);
    this.ranged = range;
    if (!range) this.dropRange();
    else if (moved && this.reviewShown) void this.loadRange(range);
  }

  /** The review screen came up: the range on view is read from now on. */
  follow() {
    this.reviewShown = true;
    if (this.ranged) void this.loadRange(this.ranged);
  }

  /** The review screen went away. */
  unfollow() {
    this.reviewShown = false;
    this.dropRange();
  }

  /** Reads the threads again. */
  async reload() {
    if (this.host.ended()) return;
    const range = this.reviewShown ? this.ranged : undefined;
    await Promise.all([this.loadWhole(), range && this.loadRange(range)]);
  }

  /** Drops everything held, for a session that ended. */
  clear() {
    this.wholeReads.cancel();
    this.rangeReads.cancel();
    this.whole = this.range = this.ranged = undefined;
    this.listeners.clear();
  }

  /** Opens a thread; resolves whether it went through. `admit` is the view `shown` is of, which says whether it still is the one asked about; by default the view on screen. */
  async create(
    shown: Shown,
    target: ThreadTarget,
    body: string,
    admit: (shown: Shown) => boolean = (sent) => this.host.admit(sent),
  ): Promise<boolean> {
    if (!admit(shown)) return false;
    return this.change(
      () => this.host.backend().createThread(shown.revision, target, body, shown.view),
      shown.revision,
    );
  }

  async reply(thread: number, body: string): Promise<boolean> {
    return this.change(() => this.host.backend().reply(thread, body));
  }

  async setStatus(thread: number, status: ThreadStatus): Promise<boolean> {
    return this.change(() => this.host.backend().setThreadStatus(thread, status));
  }

  /** Also how a detached thread is discarded. */
  async remove(thread: number): Promise<boolean> {
    if (this.host.ended()) return false;
    try {
      await this.host.backend().deleteThread(thread);
    } catch (error) {
      this.fail(error);
      return false;
    }
    if (this.host.ended()) return false;
    for (const which of CELLS) {
      const cell = this.cellOf(which);
      if (cell) this.put(which, { ...cell, items: cell.items.filter((t) => t.id !== thread) });
    }
    return true;
  }

  /** Edits one of the viewer's own comments. */
  async editMessage(message: number, body: string): Promise<boolean> {
    return this.change(() => this.host.backend().editMessage(message, body));
  }

  /** Settles a comment that was edited here and on the platform. */
  async keepEdit(message: number, keep: EditChoice): Promise<boolean> {
    return this.change(() => this.host.backend().keepEdit(message, keep));
  }

  /** Removes one of the viewer's own comments; the thread goes with its first. */
  async removeMessage(message: number): Promise<boolean> {
    if (this.host.ended()) return false;
    try {
      await this.host.backend().removeMessage(message);
    } catch (error) {
      this.fail(error);
      return false;
    }
    await this.reload();
    return true;
  }

  /** Re-anchors a moved thread where it is now. */
  async confirm(thread: number): Promise<boolean> {
    const revision = this.listed?.revision;
    return revision === undefined
      ? false
      : this.change(() => this.host.backend().confirmAnchor(revision, thread), revision);
  }

  /** Re-anchors an ambiguous thread at one of its candidates. */
  async attach(thread: number, candidate: number): Promise<boolean> {
    const revision = this.listed?.revision;
    return revision === undefined
      ? false
      : this.change(() => this.host.backend().attachAnchor(revision, thread, candidate), revision);
  }

  /** Called after every change of a list of threads. */
  onChange(listener: () => void): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  /** The cell `items` lists. */
  private get listed(): ThreadCell | undefined {
    return cellListed({ whole: this.whole, range: this.range }, this.ranged);
  }

  private loadWhole(): Promise<unknown> {
    return this.wholeReads.load(() => this.read(CHANGESET, ""));
  }

  private loadRange(range: Where): Promise<unknown> {
    return this.rangeReads.load(() => this.read(rangeView(range), rangeKey(range)));
  }

  private async read(view: Parameters<Backend["threads"]>[0], key: string): Promise<ThreadCell> {
    const { revision, threads } = await this.host.backend().threads(view);
    return { revision, key, items: threads };
  }

  private dropRange() {
    this.rangeReads.cancel();
    if (this.range) this.range = undefined;
  }

  /** A read's answer, unless the session ended or the cell holds a newer revision. */
  private take(which: Which, cell: ThreadCell) {
    if (this.host.ended() || !replaces(this.cellOf(which), cell)) return;
    // The range moved on while the read was underway.
    if (which === "range" && cell.key !== rangeKey(this.ranged)) return;
    this.put(which, cell);
  }

  private cellOf(which: Which): ThreadCell | undefined {
    return which === "whole" ? this.whole : this.range;
  }

  private put(which: Which, cell: ThreadCell) {
    if (which === "whole") this.whole = cell;
    else this.range = cell;
    for (const listener of this.listeners) listener();
  }

  /** Puts the thread a request returned into the lists that hold it, and into the one beside the view. `sent` is the revision the request named, if it did. */
  private async change(request: () => Promise<Thread>, sent?: number): Promise<boolean> {
    if (this.host.ended()) return false;
    try {
      const reply = await request();
      if (this.host.ended()) return false;
      const thread = atRevision(this.listed?.revision, reply);
      if (!thread) {
        // The changeset moved while the request was underway: the thread's anchor is of the old text.
        void this.reload();
        return false;
      }
      for (const which of CELLS) {
        const cell = this.cellOf(which);
        if (cell?.revision !== thread.revision) continue;
        const known = cell.items.some((t) => t.id === thread.id);
        if (!known && cell !== this.listed) continue;
        this.put(which, {
          ...cell,
          items: known ? cell.items.map((t) => (t.id === thread.id ? thread : t)) : [...cell.items, thread],
        });
      }
      return true;
    } catch (error) {
      if (this.host.ended()) return false;
      this.fail(error, sent);
      return false;
    }
  }

  private failed(error: unknown) {
    if (!this.host.ended()) notices.failed("Could not load comments", error);
  }

  /** A rejected request is normally a stale revision: the threads are fetched again and the viewer repeats the action. */
  private fail(error: unknown, sent?: number) {
    this.host.reportFailure(error, sent);
    void this.reload();
  }
}

export type SessionThreads = Threads;

export const newThreads = (host: ThreadsHost): SessionThreads => new Threads(host);
