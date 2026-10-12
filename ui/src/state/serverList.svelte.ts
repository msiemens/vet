// The open repository's language servers, for the footer to name the one of the focused file and
// for commands to know what it offers. Settings loads its own copy, with the installs it drives.

import type { ServerInfo } from "../generated/ServerInfo";
import { errorText } from "../lib/errorText";
import { session } from "./session.svelte";

class ServerList {
  servers = $state.raw<ServerInfo[]>([]);

  constructor() {
    session.onConnect((backend) =>
      backend.onEvent((event) => {
        if (event.project !== backend.project) return;
        // A server that has started has told what it offers; indexing is the last step of a start.
        const started = event.type === "task" && event.kind === "serverIndex" && event.state.type === "finished";
        if (event.type === "serversChanged" || started) void this.load();
      }),
    );
    session.onOpen(() => void this.load());
    session.onResync(() => {
      if (session.repo !== undefined) void this.load();
    });
    session.onClose(() => (this.servers = []));
  }

  /** Restarts the servers of `language` in the open project; resolves with why they did not start, if so. */
  async restart(language: string): Promise<string | undefined> {
    const backend = session.backend;
    if (!backend) return "No project is open";
    try {
      this.servers = await backend.restartServer(language);
      return undefined;
    } catch (error) {
      return errorText(error);
    }
  }

  private async load() {
    const backend = session.backend;
    if (!backend) return;
    try {
      this.servers = await backend.listServers();
    } catch (error) {
      console.error(error);
    }
  }
}

export const serverList = new ServerList();
