// The backend's own settings (SPEC §8) and how much disk the semantic cache takes. They belong to
// the backend, not to a project, so they can be read and changed without an open repository.

import type { BackendSettings } from "../generated/BackendSettings";
import { errorText } from "../lib/errorText";
import { session } from "./session.svelte";

class BackendSettingsStore {
  settings = $state.raw<BackendSettings>();
  loadError = $state<string>();
  /** Bytes the semantic cache's file takes on disk, as last asked. */
  usage = $state<number>();
  /** The semantic caches are being cleared. */
  clearing = $state(false);

  constructor() {
    // Another window changing them is heard once this one has read them.
    session.onConnect((backend) => backend.onSettingsChanged(() => this.settings && void this.load()));
  }

  /** Reads the settings and the cache's size again. */
  async load() {
    const backend = session.backend;
    if (!backend) return;
    try {
      [this.settings, this.usage] = await Promise.all([backend.settings(), backend.semanticCacheUsage()]);
      this.loadError = undefined;
    } catch (e) {
      this.loadError = errorText(e);
    }
  }

  /** Changes some of the settings; resolves with the backend's message if it refused them. */
  async change(change: Partial<BackendSettings>): Promise<string | undefined> {
    const backend = session.backend;
    if (!backend || !this.settings) return "Not connected to the backend.";
    try {
      this.settings = await backend.setSettings({ ...this.settings, ...change });
      // A smaller budget shrinks the file at once.
      this.usage = await backend.semanticCacheUsage();
      return undefined;
    } catch (e) {
      return errorText(e);
    }
  }

  /** Clears the backend's semantic caches; resolves with its message if that failed. */
  async clearCaches(): Promise<string | undefined> {
    const backend = session.backend;
    if (!backend) return "Not connected to the backend.";
    this.clearing = true;
    try {
      this.usage = await backend.clearSemanticCaches();
      return undefined;
    } catch (e) {
      return errorText(e);
    } finally {
      this.clearing = false;
    }
  }
}

export const backendSettings = new BackendSettingsStore();
