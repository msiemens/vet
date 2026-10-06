// What the backend sent about the open changeset. Progress and review state are the
// backend's; this holds and distributes them (SPEC I5).

import type { ReviewUpdate } from "../generated/ReviewUpdate";
import type { Manifest } from "../generated/Manifest";
import type { NonTextKind } from "../generated/NonTextKind";
import { Backend } from "./backend";

type ReviewListener = (update: ReviewUpdate) => void;

class ChangesetStore {
  status = $state("connecting…");
  backend = $state.raw<Backend>();
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
    this.applyReview(await this.backend!.acknowledge(this.manifest!.revision, path, kind, acknowledged));
  }

  applyReview(update: ReviewUpdate) {
    const manifest = this.manifest;
    if (!manifest) return;
    this.manifest = {
      ...manifest,
      completion: update.changeset,
      files: manifest.files.map((f) => (f.path === update.path ? { ...f, review: update.review, completion: update.completion } : f)),
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
