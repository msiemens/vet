// Which changeset is on view. The holder owns the session of that changeset, makes the next one when the
// backend announces it, ends the one before at once, and routes the project's events to the session. The
// switching itself is `OpenedProtocol`.

import { untrack } from "svelte";
import type { ChangesetKey } from "../generated/ChangesetKey";
import { keyId } from "./changesetKey";
import { type ChangesetSession, type SessionStart, newSession } from "./changesetSession.svelte";
import { notices } from "./notices.svelte";
import { type Opening, OpenedProtocol } from "./openedProtocol";
import { reviewShown } from "./reviewShown.svelte";
import { session as connection } from "./session.svelte";
import { CHANGESET } from "./view";
import type { ViewPoint } from "./viewChange";

class Opened {
  /** The session of the changeset on view; none while another loads, before the first is read and with no project open. */
  session: ChangesetSession | undefined = $state.raw();
  /** An open request is out. */
  private asking = $state(false);
  /** Manifests being read to build a session from. */
  private reading = $state(0);
  /** What the views were last told about, which the next session's first change is told against. */
  private viewed?: ViewPoint;
  private waiting: ((session: ChangesetSession | undefined) => void)[] = [];
  private openingListeners = new Set<(opening: Opening) => void>();
  private sessionListeners = new Set<(session: ChangesetSession | undefined) => void>();
  private protocol = new OpenedProtocol({
    dispose: () => this.drop(),
    build: (key) => this.make(key),
    start: (revision) => this.session?.start(revision),
    notify: (opening) => {
      for (const listener of this.openingListeners) listener(opening);
    },
    busy: (on) => (this.asking = on),
    read: () => this.read(),
  });

  constructor() {
    connection.onConnect((backend) =>
      backend.onEvent((event) => {
        if (event.project !== backend.project) return;
        if (event.type === "changesetOpened") this.protocol.announced(event.changeset, event.revision);
        else this.session?.hear(event);
      }),
    );
    // The changeset on view is read for the queue as well: the review screen is not what loads it.
    connection.onOpen(() => void this.read());
    connection.onResync(() => {
      if (this.session) void this.read();
    });
    connection.onClose(() => {
      this.protocol.close();
      this.viewed = undefined;
      this.release();
    });
    // The backend is told the review screen shows only once the changeset it will show is on view.
    $effect.root(() => {
      $effect(() => {
        const loading = this.loading;
        untrack(() => (reviewShown.onView = !loading));
      });
    });
  }

  /** The first manifest of a project or of the changeset just opened is being fetched: nothing of the changeset is known. */
  get loading(): boolean {
    return this.asking || this.reading > 0 || (this.session?.loading ?? false);
  }

  /**
   * Puts a changeset on view. What is on view belongs to another changeset from now on and is dropped; the
   * backend pushes `changesetOpened`, which builds the next session. If it answers without (the changeset
   * is on view already, or it refused), the manifest is read and the session built from it.
   */
  async open(key: ChangesetKey) {
    const backend = connection.connected();
    const token = this.protocol.request(key);
    try {
      await backend.openChangeset(key);
    } catch (error) {
      notices.report(error);
    }
    await this.protocol.answered(token);
  }

  /**
   * Called when a changeset is asked for (whatever belongs to the changeset on view is to be dropped
   * now), when the backend announced the one on view (to be read anew; not an earlier request's while a
   * newer is pending) and when it answered without announcing one (the changeset on view stays, to be
   * read again). Returns the unsubscribe function.
   */
  onOpening(listener: (opening: Opening) => void): () => void {
    this.openingListeners.add(listener);
    return () => this.openingListeners.delete(listener);
  }

  /** Called with the session on view whenever it is replaced, and with none when it ends. Returns the unsubscribe function. */
  onSession(listener: (session: ChangesetSession | undefined) => void): () => void {
    this.sessionListeners.add(listener);
    return () => this.sessionListeners.delete(listener);
  }

  /** The session on view, once the one being opened or read is there; none if that came to nothing. */
  current(): Promise<ChangesetSession | undefined> {
    if (this.session || !this.pending()) return Promise.resolve(this.session);
    return new Promise((resolve) => this.waiting.push(resolve));
  }

  private pending(): boolean {
    return this.asking || this.reading > 0;
  }

  /** Answers those who wait for a session when there is one or nothing is coming. */
  private release() {
    if (!this.session && this.pending()) return;
    for (const resolve of this.waiting.splice(0)) resolve(this.session);
  }

  /** The session of a changeset on view ends: it counts for nothing from now on. */
  private drop() {
    const ended = this.session;
    if (!ended) return;
    this.viewed = ended.viewed;
    ended.dispose();
    this.set(undefined);
  }

  /** The session of `key` replaces the one on view. */
  private make(key: ChangesetKey, manifest?: SessionStart["manifest"]) {
    this.set(newSession({ key, manifest, viewed: this.viewed }));
  }

  private set(next: ChangesetSession | undefined) {
    this.session = next;
    for (const listener of this.sessionListeners) listener(next);
    this.release();
  }

  /**
   * Reads the changeset the backend has on view and builds the session from it, or has the session of
   * the same changeset read again. Not while something was asked for, announced or closed meanwhile.
   */
  private async read() {
    const ticket = this.protocol.beginRead();
    this.reading++;
    try {
      const manifest = await connection.connected().manifest(CHANGESET);
      if (!this.protocol.readCurrent(ticket)) return;
      const key = manifest.target.changeset;
      const held = this.session;
      if (held && keyId(held.key) === keyId(key)) {
        held.resync(manifest);
        return;
      }
      this.drop();
      this.make(key, manifest);
      this.session?.start();
    } catch (error) {
      if (this.protocol.readCurrent(ticket)) notices.report(error);
    } finally {
      this.reading--;
      this.release();
    }
  }
}

export const opened = new Opened();
