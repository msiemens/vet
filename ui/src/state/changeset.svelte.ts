// What the backend sent about the open changeset. Progress and review state are the
// backend's; this holds and distributes them (SPEC I5).

import { untrack } from "svelte";
import { SvelteMap } from "svelte/reactivity";
import type { FileChange } from "../generated/FileChange";
import type { ReviewUpdate } from "../generated/ReviewUpdate";
import type { Manifest } from "../generated/Manifest";
import type { RestoreFailure } from "../generated/RestoreFailure";
import type { ReviewFilter } from "../generated/ReviewFilter";
import type { NonTextKind } from "../generated/NonTextKind";
import type { BaseChoice } from "../generated/BaseChoice";
import { errorText } from "../lib/errorText";
import type { BaseCandidates } from "./backend";
import { session } from "./session.svelte";

type ReviewListener = (update: ReviewUpdate) => void;

class ChangesetStore {
  /**
   * The manifest of the last refresh, with the changeset's completion kept current. Its
   * `files` are as of the refresh (paths and hunks, which only a new revision changes);
   * their review state is current in `file(path)`. Immutable snapshots, never deep proxies.
   */
  manifest = $state.raw<Manifest>();
  /** Dependency restores that failed: the analysis of those projects is limited. Pushed as they happen. */
  restoreFailures = $state.raw<RestoreFailure[]>([]);
  /** The current state of each file, one reactive cell per path: a review update invalidates only its file. */
  private cells = new SvelteMap<string, FileChange>();
  private listeners = new Set<ReviewListener>();

  files = $derived(this.manifest?.files ?? []);
  /** The lines the shown files add and remove. */
  delta = $derived(this.manifest?.delta ?? { added: 0, removed: 0 });
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

  constructor() {
    session.onConnect((backend) =>
      backend.onEvent((event) => {
        if (event.session !== backend.session) return;
        if (event.type === "snapshotSettled") void this.refresh();
        else if (event.type === "reviewChanged" && event.revision === this.manifest?.revision)
          this.applyReview(event.update);
        else if (event.type === "reviewStarted" && this.manifest)
          this.manifest = { ...this.manifest, review: event.review };
        else if (event.type === "restoreFailed") this.restoreFailures = event.failures;
      }),
    );
    session.onClose(() => {
      this.manifest = undefined;
      this.restoreFailures = [];
      this.cells.clear();
    });
  }

  /** Fetches the manifest again; the revision may have moved. */
  async refresh(): Promise<Manifest> {
    const manifest = await session.connected().manifest();
    this.cells.clear();
    for (const file of manifest.files) this.cells.set(file.path, file);
    this.manifest = manifest;
    this.restoreFailures = manifest.restoreFailures;
    return manifest;
  }

  /** Flips one hunk. `revision` is the manifest the caller displays; the backend rejects a stale one. */
  async toggleHunk(revision: number, path: string, hunk: number) {
    await this.review(revision, session.connected().toggleHunk(revision, path, hunk));
  }

  /** Flips the acknowledgement of one non-text item. */
  async toggleItem(revision: number, path: string, kind: NonTextKind) {
    await this.review(revision, session.connected().toggleItem(revision, path, kind));
  }

  /** Marks everything of a file seen, or takes every mark back if it all is seen already. */
  async toggleFileSeen(revision: number, path: string) {
    await this.review(revision, session.connected().toggleFileSeen(revision, path));
  }

  /**
   * Compares against `choice`. The backend remaps the review state and pushes
   * `snapshotSettled`, which fetches the new manifest; a choice that does not resolve shows the
   * backend's message.
   */
  async setBase(choice: BaseChoice) {
    try {
      await session.connected().setBase(choice);
    } catch (error) {
      session.notice = errorText(error);
    }
  }

  /**
   * Ends the open review and says how it ended. The backend pushes `snapshotSettled`, which
   * fetches the manifest again, back to browsing.
   */
  async finishReview() {
    try {
      const complete = await session.connected().finishReview();
      session.notice = complete ? "Review finished: everything was seen" : "Review finished with unseen changes left";
    } catch (error) {
      session.notice = errorText(error);
    }
  }

  /**
   * Replaces the repository's review filter. The backend pushes `snapshotSettled`, which fetches the new
   * manifest. Resolves with the backend's message if it refused the filter (a bad glob).
   */
  async setFilter(filter: ReviewFilter): Promise<string | undefined> {
    try {
      await session.connected().setProjectFilter(filter);
      return undefined;
    } catch (error) {
      return errorText(error);
    }
  }

  /** Excludes `path` (a file, or a folder with `dir`) from the review. As with `setFilter`, the backend then pushes `snapshotSettled`. */
  async excludePath(path: string, dir: boolean) {
    try {
      await session.connected().excludePath(path, dir);
    } catch (error) {
      session.notice = errorText(error);
    }
  }

  /** Limits the review to the folder `path`. */
  async reviewOnly(path: string) {
    const filter = this.manifest?.filter;
    if (!filter) return;
    const error = await this.setFilter({ ...filter, roots: [path] });
    if (error) session.notice = error;
  }

  /** The children of a directory of the target snapshot; `undefined` after showing why it failed. */
  async listDirectory(path: string) {
    try {
      return await session.connected().listDirectory(path);
    } catch (error) {
      session.notice = errorText(error);
      return undefined;
    }
  }

  /** What a base can be picked from; `undefined` after showing why it failed. */
  async baseCandidates(): Promise<BaseCandidates | undefined> {
    try {
      return await session.connected().baseCandidates();
    } catch (error) {
      session.notice = errorText(error);
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
      await this.refresh().catch(() => undefined);
      this.reportFailure(error, sent);
    }
  }

  /**
   * Tells the viewer why a request failed. When the manifest has moved on from the revision the
   * request was sent for, the views are about to be replaced and the action has to be repeated.
   */
  reportFailure(error: unknown, sent?: number) {
    const current = this.manifest?.revision;
    session.notice =
      sent !== undefined && current !== undefined && current !== sent
        ? "Changes arrived — try again"
        : `Request failed: ${errorText(error)}`;
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
