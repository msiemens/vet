// What the backend sent about the open changeset, plus the display state derived from it.

import type { FileChange } from "../generated/FileChange";
import type { FileReview } from "../generated/FileReview";
import type { Manifest } from "../generated/Manifest";
import type { NonTextKind } from "../generated/NonTextKind";
import { Backend } from "./backend";

/** Changed lines and non-text items of a file in total, seen/acknowledged, and seen against an old base. */
export function coverage(f: FileChange) {
  let total = f.nonText.length;
  let seen = f.review.acknowledged.length;
  let old = 0;
  f.hunks.forEach((h, i) => {
    const lines = h.baseLen + h.targetLen;
    total += lines;
    const state = f.review.hunks[i];
    if (state === "seen") seen += lines;
    else if (state === "seenAgainstOldBase") old += lines;
  });
  return { total, seen, old };
}

type ReviewListener = (path: string, review: FileReview) => void;

class ChangesetStore {
  status = $state("connecting…");
  backend = $state.raw<Backend>();
  manifest = $state.raw<Manifest>();
  private listeners = new Set<ReviewListener>();

  files = $derived(this.manifest?.files ?? []);

  /** Share of changed lines and non-text items seen, and seen against an old base, in percent. */
  progress = $derived.by(() => {
    let total = 0;
    let seen = 0;
    let old = 0;
    for (const f of this.files) {
      const c = coverage(f);
      total += c.total;
      seen += c.seen;
      old += c.old;
    }
    return { total, seen, old, seenPct: total ? (seen / total) * 100 : 0, oldPct: total ? (old / total) * 100 : 0 };
  });

  async connect() {
    try {
      const backend = await Backend.connect();
      backend.onEvent((event) => {
        if (event.session !== backend.session) return;
        if (event.type === "snapshotSettled") void this.refresh();
        else if (event.revision === this.manifest?.revision) this.applyReview(event.path, event.review);
      });
      this.backend = backend;
    } catch (error) {
      this.status = (error as Error).message;
    }
  }

  /** Fetches the manifest again; the revision may have moved. */
  async refresh(): Promise<Manifest> {
    const manifest = await this.backend!.manifest();
    this.manifest = manifest;
    return manifest;
  }

  async acknowledge(path: string, kind: NonTextKind, acknowledged: boolean) {
    const review = await this.backend!.acknowledge(this.manifest!.revision, path, kind, acknowledged);
    this.applyReview(path, review);
  }

  applyReview(path: string, review: FileReview) {
    const manifest = this.manifest;
    if (!manifest) return;
    this.manifest = { ...manifest, files: manifest.files.map((f) => (f.path === path ? { ...f, review } : f)) };
    for (const listener of this.listeners) listener(path, review);
  }

  /** Called after each review update, whether it came from a request or an event. */
  onReview(listener: ReviewListener): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }
}

export const changeset = new ChangesetStore();
