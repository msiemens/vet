// The recently opened repositories, kept by the backend.

import type { RecentProject } from "../generated/RecentProject";
import type { Backend } from "./backend";
import { Latest } from "./resource";

class RecentRepositories {
  /** Backend-side paths with when they were opened, most recent first. */
  list = $state<RecentProject[]>([]);
  /** Only the latest answer counts. */
  private reads = new Latest();

  /** Loads the list and keeps it current: this window and others change it. */
  async start(backend: Backend) {
    backend.onRecentProjectsChanged(() => void this.load(backend));
    await this.load(backend);
  }

  /** Reads the list again; `Subscribe` is what hears a change of it. */
  async load(backend: Backend) {
    await this.take(backend.recentProjects().catch(() => []));
  }

  async remember(backend: Backend, path: string) {
    await this.take(
      backend.recordRecentProject(path).catch((e: unknown) => {
        console.error(e);
        return this.list;
      }),
    );
  }

  async forget(backend: Backend, path: string) {
    await this.take(
      backend.forgetRecentProject(path).catch((e: unknown) => {
        console.error(e);
        return this.list;
      }),
    );
  }

  private async take(answer: Promise<RecentProject[]>) {
    const isLatest = this.reads.begin();
    const list = await answer;
    if (isLatest()) this.list = list;
  }
}

export const recent = new RecentRepositories();
