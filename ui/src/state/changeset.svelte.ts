// What the backend sent about the open changeset. Progress and review state are the
// backend's; this holds and distributes them (SPEC I5).

import type { ReviewUpdate } from "../generated/ReviewUpdate";
import type { Manifest } from "../generated/Manifest";
import type { NonTextKind } from "../generated/NonTextKind";
import { Backend } from "./backend";
import { canPickFolder, pickRepository } from "./host";
import { recent } from "./recent.svelte";

type ReviewListener = (update: ReviewUpdate) => void;

class ChangesetStore {
  status = $state("connecting…");
  /** A short message for the viewer about a request that did not go through. */
  notice = $state<string>();
  backend = $state.raw<Backend>();
  /** The root of the open repository; none shows the start screen. */
  repo = $state<string>();
  /** Why the last attempt to open a repository failed. */
  openError = $state<{ path: string; message: string }>();
  manifest = $state.raw<Manifest>();
  private listeners = new Set<ReviewListener>();

  files = $derived(this.manifest?.files ?? []);

  async connect() {
    try {
      const backend = await Backend.connect();
      backend.onEvent((event) => {
        if (event.session !== backend.session) return;
        if (event.type === "snapshotSettled") void this.refresh();
        else if (event.revision === this.manifest?.revision) this.applyReview(event.update);
      });
      this.backend = backend;
      await recent.load();
      if (backend.initialRepo) await this.openRepo(backend.initialRepo);
    } catch (error) {
      this.status = (error as Error).message;
    }
  }

  /** Closes the open repository, if any, and opens `path` (the backend resolves it to a repository root). */
  async openRepo(path: string) {
    const backend = this.connected();
    this.openError = undefined;
    await this.closeRepo();
    try {
      this.repo = await backend.open(path);
      void recent.remember(this.repo);
    } catch (error) {
      this.openError = { path, message: error instanceof Error ? error.message : String(error) };
    }
  }

  /** Asks the host's folder dialog for a repository and opens it. Without a dialog (a plain browser) this goes to the start screen. */
  async pickRepo() {
    if (!canPickFolder) {
      await this.closeRepo();
      return;
    }
    try {
      const path = await pickRepository();
      if (path) await this.openRepo(path);
    } catch (error) {
      // The host refuses folders the backend cannot see (the WSL split) with a message.
      this.openError = { path: "", message: error instanceof Error ? error.message : String(error) };
    }
  }

  /** Back to the start screen. */
  async closeRepo() {
    this.repo = undefined;
    this.manifest = undefined;
    await this.connected().close();
  }

  /** Fetches the manifest again; the revision may have moved. */
  async refresh(): Promise<Manifest> {
    const manifest = await this.connected().manifest();
    this.manifest = manifest;
    return manifest;
  }

  /** `revision` is the manifest the caller displays; the backend rejects a stale one. */
  async setSeen(revision: number, path: string, hunks: number[], seen: boolean) {
    await this.review(revision, this.connected().setSeen(revision, path, hunks, seen));
  }

  async acknowledge(path: string, kind: NonTextKind, acknowledged: boolean) {
    const backend = this.connected();
    if (!this.manifest) throw new Error("no manifest loaded");
    const { revision } = this.manifest;
    await this.review(revision, backend.acknowledge(revision, path, kind, acknowledged));
  }

  /**
   * Applies the review state a request returned. A rejected request is normally a
   * stale revision: refetch the manifest and tell the viewer to repeat the action.
   */
  private async review(sent: number, request: Promise<ReviewUpdate>) {
    try {
      this.applyReview(await request);
    } catch (error) {
      const manifest = await this.refresh().catch(() => undefined);
      this.notice =
        manifest && manifest.revision !== sent
          ? "Changes arrived — try again"
          : `Request failed: ${error instanceof Error ? error.message : String(error)}`;
    }
  }

  private connected(): Backend {
    if (!this.backend) throw new Error("not connected to the backend");
    return this.backend;
  }

  applyReview(update: ReviewUpdate) {
    const manifest = this.manifest;
    if (!manifest) return;
    this.manifest = {
      ...manifest,
      completion: update.changeset,
      files: manifest.files.map((f) =>
        f.path === update.path ? { ...f, review: update.review, completion: update.completion } : f,
      ),
    };
    for (const listener of this.listeners) listener(update);
  }

  /** Called after each review update, whether it came from a request or an event. */
  onReview(listener: ReviewListener): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }
}

export const changeset = new ChangesetStore();
