// The language servers of the open repository as the backend resolves them, and the installs
// the viewer started. Servers resolve per repository, so every request needs a session.

import type { ServerInfo } from "../generated/ServerInfo";
import { errorText } from "../lib/errorText";
import { type InstallProgress, appendLog } from "./installLog";
import { session } from "./session.svelte";

class ServersStore {
  servers = $state.raw<ServerInfo[]>([]);
  loading = $state(false);
  loadError = $state<string>();
  installs = $state.raw<Partial<Record<string, InstallProgress>>>({});
  /** The output of each language's install. */
  logs = $state.raw<Record<string, string[]>>({});
  /** Whether each language's install output is shown. */
  logOpen = $state.raw<Record<string, boolean>>({});

  /** There is an open repository whose session can be asked. */
  get available(): boolean {
    return session.repo !== undefined && session.backend?.session !== undefined;
  }

  /** Loads the servers and keeps them current with the backend's events; the returned function stops that. */
  start(): () => void {
    this.servers = [];
    this.loadError = undefined;
    this.installs = {};
    this.logs = {};
    this.logOpen = {};
    const backend = this.available ? session.backend : undefined;
    if (!backend) return () => undefined;
    void this.load();
    return backend.onEvent((event) => {
      if (event.session !== backend.session) return;
      if (event.type === "serverInstallOutput") {
        this.logs = { ...this.logs, [event.language]: appendLog(this.logs[event.language] ?? [], event.line) };
      } else if (event.type === "serversChanged") {
        void this.load();
      }
    });
  }

  private async load() {
    const backend = session.backend;
    if (!backend) return;
    this.loading = true;
    try {
      this.servers = await backend.listServers();
      this.loadError = undefined;
    } catch (e) {
      this.loadError = errorText(e);
    } finally {
      this.loading = false;
    }
  }

  async install(language: string) {
    const backend = session.backend;
    if (!backend) return;
    this.installs = { ...this.installs, [language]: { running: true } };
    this.logs = { ...this.logs, [language]: [] };
    this.logOpen = { ...this.logOpen, [language]: true };
    try {
      this.servers = await backend.installServer(language);
      this.installs = { ...this.installs, [language]: { running: false } };
    } catch (e) {
      this.installs = { ...this.installs, [language]: { running: false, error: errorText(e) } };
    }
  }

  /** Sets the executable of a language's server (none: back to the resolved one); resolves with the backend's message if it refused the path. */
  async setPath(language: string, path: string | null): Promise<string | undefined> {
    const backend = session.backend;
    if (!backend) return undefined;
    try {
      this.servers = await backend.setServerPath(language, path);
      // The result of an earlier install no longer describes what resolves now.
      this.installs = { ...this.installs, [language]: { running: false } };
      return undefined;
    } catch (e) {
      return errorText(e);
    }
  }

  setLogOpen(language: string, open: boolean) {
    this.logOpen = { ...this.logOpen, [language]: open };
  }
}

export const servers = new ServersStore();
