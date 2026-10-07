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
import type { ChangesetKey } from "../generated/ChangesetKey";
import type { View } from "../generated/View";
import { errorText } from "../lib/errorText";
import type { BaseCandidates } from "./backend";
import { session } from "./session.svelte";
import { type Range, followed, rangeKey, rangeView } from "./timelineRange";

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

  /**
   * The versions the review view compares instead of showing the changeset; none for the changeset.
   * Set it with `setRange`.
   */
  range = $state.raw<Range>();
  /** What requests about the files read: the interdiff of `range`, or the changeset. */
  get view(): View {
    return rangeView(this.range);
  }
  /** The range the manifest is of, as a string; empty for the changeset. */
  viewKey = $derived(rangeKey(this.manifest?.interdiff));

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
        if (event.project !== backend.project) return;
        if (event.type === "changesetOpened") {
          // Versions are the changeset's own.
          this.range = undefined;
          void this.refresh();
        } else if (event.type === "snapshotSettled") void this.refresh();
        // A mark made in a range is announced as that of the changeset's file, which the range does not show.
        else if (event.type === "reviewChanged" && !this.range && event.revision === this.manifest?.revision)
          this.applyReview(event.update);
        else if (event.type === "reviewStarted" && this.manifest)
          this.manifest = { ...this.manifest, review: event.review };
        else if (event.type === "restoreFailed") this.restoreFailures = event.failures;
        else if (event.type === "baseChanged" && this.manifest) this.manifest = { ...this.manifest, base: event.base };
      }),
    );
    session.onClose(() => {
      this.manifest = undefined;
      this.range = undefined;
      this.restoreFailures = [];
      this.cells.clear();
    });
  }

  /** Fetches the manifest again; the revision may have moved. */
  async refresh(): Promise<Manifest> {
    const manifest = await this.fetchManifest();
    this.cells.clear();
    for (const file of manifest.files) this.cells.set(file.path, file);
    this.manifest = manifest;
    this.restoreFailures = manifest.restoreFailures;
    return manifest;
  }

  /**
   * The manifest of the view. A live range is moved to the latest version first, which only the
   * manifest says; a range the backend refuses (its versions are gone) gives way to the changeset.
   */
  private async fetchManifest(): Promise<Manifest> {
    const backend = session.connected();
    for (let attempt = 0; ; attempt++) {
      const range = this.range;
      let manifest: Manifest;
      try {
        manifest = await backend.manifest(rangeView(range));
      } catch (error) {
        if (!range) throw error;
        this.range = undefined;
        session.notice = errorText(error);
        continue;
      }
      const latest = manifest.review.state === "open" ? manifest.review.version : undefined;
      if (range?.live && latest !== undefined && range.to !== latest && attempt < 3) {
        this.range = followed(range, latest);
        continue;
      }
      return manifest;
    }
  }

  /**
   * Shows the interdiff of `range`, or the changeset for none, and fetches its manifest. Only the
   * working tree has a live end: a committed changeset's latest version does not follow anything.
   */
  async setRange(range: Range | undefined) {
    const follows = this.manifest?.target.changeset.type === "workingTree";
    this.range = range && { ...range, live: range.live && follows };
    try {
      await this.refresh();
    } catch (error) {
      session.notice = errorText(error);
    }
  }

  /** Flips one hunk. `revision` is the manifest the caller displays; the backend rejects a stale one. */
  async toggleHunk(revision: number, path: string, hunk: number) {
    await this.review(revision, session.connected().toggleHunk(revision, path, hunk, this.view));
  }

  /** Flips the acknowledgement of one non-text item. */
  async toggleItem(revision: number, path: string, kind: NonTextKind) {
    await this.review(revision, session.connected().toggleItem(revision, path, kind, this.view));
  }

  /** Marks everything of a file seen, or takes every mark back if it all is seen already. */
  async toggleFileSeen(revision: number, path: string) {
    await this.review(revision, session.connected().toggleFileSeen(revision, path, this.view));
  }

  /** The changesets on offer and the one on view; `undefined` after showing why they could not be listed. */
  async listChangesets() {
    try {
      return await session.connected().listChangesets();
    } catch (error) {
      session.notice = errorText(error);
      return undefined;
    }
  }

  /** Puts a changeset on view. The backend pushes `changesetOpened`, which fetches its manifest. */
  async openChangeset(key: ChangesetKey) {
    try {
      await session.connected().openChangeset(key);
    } catch (error) {
      session.notice = errorText(error);
    }
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

  /** Fetches the remotes. The footer shows the task; the base's `behind` follows with `baseChanged`. */
  async fetch() {
    try {
      await session.connected().fetch();
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
      this.range = undefined;
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

export type Changeset = ChangesetStore;

export const changeset = new ChangesetStore();
