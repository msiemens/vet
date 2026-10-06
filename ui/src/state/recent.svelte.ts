// The recently opened repositories, kept by the host (not the backend).

import { forgetRepository, recentRepositories, rememberRepository } from "./host";

class RecentRepositories {
  list = $state<string[]>([]);

  async load() {
    this.list = await recentRepositories().catch(() => []);
  }

  async remember(path: string) {
    await rememberRepository(path).catch((e: unknown) => console.error(e));
    await this.load();
  }

  async forget(path: string) {
    await forgetRepository(path).catch((e: unknown) => console.error(e));
    await this.load();
  }
}

export const recent = new RecentRepositories();
