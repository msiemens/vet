// The window's session with the backend: the connection, the open repository and the notice
// shown to the viewer. The stores of what the repository holds (changeset, threads, semantics) take
// the backend and report failures from here.

import { errorText } from "../lib/errorText";
import { Backend } from "./backend";
import { hasNativeDialogs, pickRepository } from "./host";
import { recent } from "./recent.svelte";

class SessionStore {
  status = $state("connecting…");
  /** A short message for the viewer about a request that did not go through. */
  notice = $state<string>();
  backend = $state.raw<Backend>();
  /** The root of the open repository; none shows the welcome screen. */
  repo = $state<string>();
  /** Why the last attempt to open a repository failed. */
  openError = $state<{ path: string; message: string }>();
  private connectListeners = new Set<(backend: Backend) => void>();
  private closeListeners = new Set<() => void>();

  async connect() {
    try {
      const backend = await Backend.connect();
      for (const listener of this.connectListeners) listener(backend);
      this.backend = backend;
      await recent.load(backend);
      if (backend.initialRepo) await this.openRepo(backend.initialRepo);
    } catch (error) {
      this.status = errorText(error);
    }
  }

  /** Called with the backend once it is connected, before anything is opened. */
  onConnect(listener: (backend: Backend) => void) {
    this.connectListeners.add(listener);
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

  /** Closes the open repository, if any, and opens `path` (the backend resolves it to a repository root). */
  async openRepo(path: string) {
    const backend = this.connected();
    this.openError = undefined;
    await this.closeRepo();
    try {
      this.repo = await backend.open(path);
      void recent.remember(backend, this.repo);
    } catch (error) {
      this.openError = { path, message: errorText(error) };
    }
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

  /** Back to the welcome screen. */
  async closeRepo() {
    this.repo = undefined;
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
