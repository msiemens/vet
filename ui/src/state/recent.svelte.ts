// The recently opened repositories, kept by the backend.

import type { RecentProject } from "../generated/RecentProject";
import type { Backend } from "./backend";

class RecentRepositories {
  /** Backend-side paths with when they were opened, most recent first. */
  list = $state<RecentProject[]>([]);

  async load(backend: Backend) {
    this.list = await backend.recentProjects().catch(() => []);
  }

  async remember(backend: Backend, path: string) {
    this.list = await backend.recordRecentProject(path).catch((e: unknown) => {
      console.error(e);
      return this.list;
    });
  }

  async forget(backend: Backend, path: string) {
    this.list = await backend.forgetRecentProject(path).catch((e: unknown) => {
      console.error(e);
      return this.list;
    });
  }
}

export const recent = new RecentRepositories();
