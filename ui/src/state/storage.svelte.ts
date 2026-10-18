// What the projects take on disk, for the Storage pages of Settings. They belong to the backend, not
// to the open project: a page watches either every project or one root, and the figures follow the
// backend's events while it is watched.

import type { AppStorage } from "../generated/AppStorage";
import type { ProjectStorage } from "../generated/ProjectStorage";
import { errorText } from "../lib/errorText";
import { Latest } from "./resource";
import { session } from "./session.svelte";

/** What a page shows the figures of: every project and the app's own, or the project at `root`. */
type Watched = "all" | { root: string };

class StorageStore {
  /** Figures of every project, while they are watched; none before the first answer. */
  all = $state.raw<{ projects: ProjectStorage[]; app: AppStorage }>();
  /** Figures of the watched project; none before the first answer. */
  project = $state.raw<ProjectStorage>();
  loadError = $state<string>();
  /** The root whose caches are being cleared. */
  busy = $state<string>();
  private watched?: Watched;
  /** Only the latest answer counts. */
  private reads = new Latest();

  constructor() {
    session.onConnect((backend) => {
      const reload = () => void this.load();
      backend.onStorageChanged(reload);
      backend.onSettingsChanged(reload);
    });
    session.onResync(() => void this.load());
  }

  /** Reads the figures now and again whenever they change; returns the function that stops watching. */
  watch(what: Watched): () => void {
    this.watched = what;
    this.all = undefined;
    this.project = undefined;
    this.loadError = undefined;
    void this.load();
    return () => {
      if (this.watched !== what) return;
      this.watched = undefined;
      this.reads.cancel();
    };
  }

  /** Removes what the project at `root` keeps only for speed, and shows the figures it answers with. */
  async clear(root: string) {
    const backend = session.backend;
    if (!backend) return;
    const isLatest = this.reads.begin();
    this.busy = root;
    try {
      const project = await backend.clearProjectCaches(root);
      if (isLatest() && typeof this.watched === "object" && this.watched.root === root) {
        this.project = project;
        this.loadError = undefined;
      }
    } catch (e) {
      this.loadError = errorText(e);
    } finally {
      this.busy = undefined;
    }
  }

  private async load() {
    const what = this.watched;
    const backend = session.backend;
    if (!what || !backend) return;
    const isLatest = this.reads.begin();
    try {
      if (what === "all") {
        const all = await backend.projectsStorage();
        if (isLatest()) this.all = all;
      } else {
        const project = await backend.projectStorage(what.root);
        if (isLatest()) this.project = project;
      }
      if (isLatest()) this.loadError = undefined;
    } catch (e) {
      if (isLatest()) this.loadError = errorText(e);
    }
  }
}

export const storage = new StorageStore();
