// The open repository's language servers, for the footer to name the one of the focused file.
// Settings loads its own copy, with the installs it drives.

import type { ServerInfo } from "../generated/ServerInfo";
import { session } from "./session.svelte";

class ServerList {
  servers = $state.raw<ServerInfo[]>([]);

  constructor() {
    session.onConnect((backend) =>
      backend.onEvent((event) => {
        if (event.type === "serversChanged" && event.project === backend.project) void this.load();
      }),
    );
    session.onOpen(() => void this.load());
    session.onClose(() => (this.servers = []));
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
