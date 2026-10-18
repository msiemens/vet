// The window's session with the backend: the connection and the open repository. The stores of
// what the repository holds (changeset, threads, semantics) take the backend from here.

import { errorText } from "../lib/errorText";
import { Backend, type LinkState } from "./backend";
import { whenDiscarded } from "./discard";
import { discardPrompt } from "./discardPrompt.svelte";
import { hasNativeDialogs, pickRepository, quit, restartBackend, setWindowMode } from "./host";
import { notices } from "./notices.svelte";
import { preferenceSync } from "./preferenceSync";
import { recent } from "./recent.svelte";
import { RepoOpener } from "./repoOpener";

class SessionStore {
  status = $state("connecting…");
  backend = $state.raw<Backend>();
  /** Whether the connection to the backend is up; it connects again by itself when it drops. */
  link = $state<LinkState>("connected");
  /** The root of the open repository; none shows the welcome screen. */
  repo = $state<string>();
  /** Why the last attempt to open a repository failed. */
  openError = $state<{ path: string; message: string }>();
  /** How many comment boxes hold text that closing the project loses; the holder of the changeset on view sets it. */
  unsentComments: () => number = () => 0;
  private connectListeners = new Set<(backend: Backend) => void>();
  private openListeners = new Set<(backend: Backend) => void>();
  private closeListeners = new Set<() => void>();
  private resyncListeners = new Set<(backend: Backend) => void>();
  private opener = new RepoOpener({
    release: () => this.release(),
    open: async (path) => {
      const root = await this.connected().open(path);
      await preferenceSync.openProject(root);
      return root;
    },
    setWindowMode,
    opened: (repo) => {
      this.repo = repo;
      void recent.remember(this.connected(), repo);
      for (const listener of this.openListeners) listener(this.connected());
    },
    failed: (path, error) => (this.openError = { path, message: errorText(error) }),
  });

  async connect() {
    try {
      const backend = await Backend.connect();
      await backend.subscribe();
      await preferenceSync.start(backend);
      backend.onLinkState((state) => (this.link = state));
      backend.onReconnect(() => void this.resync(backend));
      backend.onProjectRemoved((root) => {
        if (root === this.repo) void this.removed(backend);
      });
      for (const listener of this.connectListeners) listener(backend);
      this.backend = backend;
      await recent.start(backend);
      if (backend.initialRepo) await this.openRepo(backend.initialRepo);
    } catch (error) {
      this.status = errorText(error);
    }
  }

  /**
   * The connection is up again after it dropped, maybe to a backend the host started anew: this
   * connection hears the backend's own events again, the open repository is opened again on it,
   * which subscribes to its events, and every store reads anew.
   */
  private async resync(backend: Backend) {
    const repo = this.repo;
    await backend.subscribe();
    if (repo !== undefined) {
      try {
        await backend.open(repo);
      } catch (error) {
        await this.abandon(error);
        return;
      }
    }
    void recent.load(backend);
    void preferenceSync.load(backend).catch(console.error);
    for (const listener of this.resyncListeners) listener(backend);
  }

  /** Called with the backend once it is connected, before anything is opened. */
  onConnect(listener: (backend: Backend) => void) {
    this.connectListeners.add(listener);
  }

  /**
   * Called when the connection is up again after it dropped and the open repository, if any, is open
   * on it again. What a store holds may be out of date by then: it reads it again. Returns the
   * unsubscribe function.
   */
  onResync(listener: (backend: Backend) => void): () => void {
    this.resyncListeners.add(listener);
    return () => this.resyncListeners.delete(listener);
  }

  /** Ends the backend and has the host start another, for when the connection does not come back. */
  async restartBackend() {
    try {
      await restartBackend();
      this.connected().retryConnection();
    } catch (error) {
      notices.failed("Could not restart the backend", error);
    }
  }

  /**
   * Called when a repository is open on the backend and the window shows it, which is before the
   * backend has read it: events about it may have arrived already.
   */
  onOpen(listener: (backend: Backend) => void) {
    this.openListeners.add(listener);
  }

  /** Called when the open repository is closed, before the backend is told. */
  onClose(listener: () => void) {
    this.closeListeners.add(listener);
  }

  /** The backend; throws before the connection is up. */
  connected(): Backend {
    if (!this.backend) throw new Error("not connected to the backend");
    return this.backend;
  }

  /**
   * Closes the open repository, if any, and opens `path` (the backend resolves it to a repository root).
   * Asking for a path that is being opened joins that open: a double click or a held Enter opens once.
   */
  async openRepo(path: string) {
    this.connected();
    await whenDiscarded(this.unsentComments(), discardPrompt.ask, async () => {
      this.openError = undefined;
      await this.opener.open(path);
    });
  }

  /**
   * Asks the host's folder dialog for a repository; resolves with its path, or nothing if cancelled.
   * Without a dialog (a plain browser) this goes to the welcome screen.
   */
  async pickRepo(): Promise<string | undefined> {
    if (!hasNativeDialogs) {
      await this.closeRepo();
      return undefined;
    }
    try {
      return (await pickRepository()) ?? undefined;
    } catch (error) {
      // The host refuses folders the backend cannot see (the WSL split) with a message.
      this.openError = { path: "", message: errorText(error) };
      return undefined;
    }
  }

  /** Back to the welcome screen, in the welcome screen's window. Comment text not added is asked about first. */
  async closeRepo() {
    await whenDiscarded(this.unsentComments(), discardPrompt.ask, () => this.leave());
  }

  /** Ends the app, after asking about comment text not added. */
  async quit() {
    await whenDiscarded(this.unsentComments(), discardPrompt.ask, quit);
  }

  private async leave() {
    await this.release();
    await setWindowMode("welcome");
  }

  /** The open repository was removed, by this window or another: the backend shut it already, so there is nothing to ask or close. */
  private async removed(backend: Backend) {
    backend.dropProject();
    await this.leave();
  }

  /**
   * The open repository turned out not to be readable (the backend opens before it reads): back to
   * the welcome screen, which says why.
   */
  async abandon(error: unknown) {
    const path = this.repo ?? "";
    await this.leave();
    this.openError = { path, message: errorText(error) };
  }

  /** Closes the open repository without touching the window, for a caller that opens another. */
  private async release() {
    this.repo = undefined;
    preferenceSync.closeProject();
    for (const listener of this.closeListeners) listener();
    await this.connected().close();
  }

  /** Drops `path` from the recent list, and the failure that was about it. */
  async forget(path: string) {
    this.openError = undefined;
    await recent.forget(this.connected(), path);
  }
}

export const session = new SessionStore();
