// What the backend sent about the open changeset. Progress and review state are the
// backend's; this holds and distributes them (SPEC I5).

import { untrack } from "svelte";
import { SvelteMap } from "svelte/reactivity";
import type { FileChange } from "../generated/FileChange";
import type { ReviewUpdate } from "../generated/ReviewUpdate";
import type { Manifest } from "../generated/Manifest";
import type { NonTextKind } from "../generated/NonTextKind";
import type { BaseChoice } from "../generated/BaseChoice";
import { Backend, type BaseCandidates } from "./backend";
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
  /**
   * The manifest of the last refresh, with the changeset's completion kept current. Its
   * `files` are as of the refresh (paths and hunks, which only a new revision changes);
   * their review state is current in `file(path)`. Immutable snapshots, never deep proxies.
   */
  manifest = $state.raw<Manifest>();
  /** The current state of each file, one reactive cell per path: a review update invalidates only its file. */
  private cells = new SvelteMap<string, FileChange>();
  private listeners = new Set<ReviewListener>();

  files = $derived(this.manifest?.files ?? []);
  /** Same array while only review state changes. */
  paths = $derived(this.files.map((f) => f.path));

  /** A file as it is now, with its current review state and completion. */
  file(path: string): FileChange | undefined {
    return this.cells.get(path);
  }

  /** The manifest with every file's current state, for building views; reads nothing reactively. */
  currentManifest(): Manifest | undefined {
    return untrack(() => {
      const manifest = this.manifest;
      return manifest && { ...manifest, files: manifest.files.map((f) => this.cells.get(f.path) ?? f) };
    });
  }

  async connect() {
    try {
      const backend = await Backend.connect();
      backend.onEvent((event) => {
        if (event.session !== backend.session) return;
        if (event.type === "snapshotSettled") void this.refresh();
        else if (event.type === "reviewChanged" && event.revision === this.manifest?.revision)
          this.applyReview(event.update);
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
    this.cells.clear();
    await this.connected().close();
  }

  /** Fetches the manifest again; the revision may have moved. */
  async refresh(): Promise<Manifest> {
    const manifest = await this.connected().manifest();
    this.cells.clear();
    for (const file of manifest.files) this.cells.set(file.path, file);
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
   * Compares against `choice`. The backend remaps the review state and pushes
   * `snapshotSettled`, which fetches the new manifest; a choice that does not resolve shows the
   * backend's message.
   */
  async setBase(choice: BaseChoice) {
    try {
      await this.connected().setBase(choice);
    } catch (error) {
      this.notice = error instanceof Error ? error.message : String(error);
    }
  }

  /** What a base can be picked from; `undefined` after showing why it failed. */
  async baseCandidates(): Promise<BaseCandidates | undefined> {
    try {
      return await this.connected().baseCandidates();
    } catch (error) {
      this.notice = error instanceof Error ? error.message : String(error);
      return undefined;
    }
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
    const file = this.cells.get(update.path);
    if (file) this.cells.set(update.path, { ...file, review: update.review, completion: update.completion });
    this.manifest = { ...manifest, completion: update.changeset };
    for (const listener of this.listeners) listener(update);
  }

  /** Called after each review update, whether it came from a request or an event. */
  onReview(listener: ReviewListener): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }
}

export const changeset = new ChangesetStore();
