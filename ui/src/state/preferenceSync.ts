// Keeps `lib/preferences` and the backend's preferences the same: loads them, sends the viewer's
// changes and takes those that other windows announce.

import { type Scope, preferences } from "../lib/preferences.svelte";
import type { Backend } from "./backend";

class PreferenceSync {
  private backend: Backend | undefined;
  /** The root of the project whose preferences are loaded. */
  private repo: string | undefined;
  /** Changes sent and not yet answered, by scope and key: their own announcements are not news. */
  private sending = new Map<string, number>();

  /** Connects the preferences to `backend` and loads the app's. */
  async start(backend: Backend) {
    this.backend = backend;
    preferences.sink = (scope, key, value) => this.send(backend, scope, key, value);
    backend.onPreferenceChanged((scope, key, value) => this.heard(scope, key, value));
    await this.load(backend);
  }

  /** Loads the preferences of the app and of the open project again; the connection is up. */
  async load(backend: Backend) {
    await Promise.all([
      this.take(backend, "app", null),
      this.repo === undefined ? undefined : this.take(backend, "project", this.repo),
    ]);
  }

  /** Loads the preferences of the project with the repository root `repo`. */
  async openProject(repo: string) {
    this.repo = repo;
    if (this.backend) await this.take(this.backend, "project", repo);
  }

  closeProject() {
    this.repo = undefined;
    preferences.replace("project", []);
  }

  private async take(backend: Backend, scope: Scope, root: string | null) {
    const values = await backend.preferences(root);
    preferences.replace(
      scope,
      values.map((p) => [p.key, p.value]),
    );
  }

  private send(backend: Backend, scope: Scope, key: string, value: string | undefined) {
    const root = scope === "app" ? null : this.repo;
    if (root === undefined) return;
    const id = `${scope}:${key}`;
    this.sending.set(id, (this.sending.get(id) ?? 0) + 1);
    backend
      .setPreference(root, key, value)
      .catch((e: unknown) => console.error(e))
      .finally(() => {
        const left = (this.sending.get(id) ?? 1) - 1;
        if (left > 0) this.sending.set(id, left);
        else this.sending.delete(id);
      });
  }

  private heard(root: string | null, key: string, value: string | undefined) {
    const scope: Scope | undefined = root === null ? "app" : root === this.repo ? "project" : undefined;
    if (scope === undefined || this.sending.has(`${scope}:${key}`)) return;
    preferences.keep(scope, key, value);
  }
}

export const preferenceSync = new PreferenceSync();
