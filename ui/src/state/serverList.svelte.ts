// The open repository's language servers as the backend resolves them, for the footer to name the
// one of the focused file, for commands to know what it offers and for the Servers page to list,
// with the installs the viewer started and their output. What the servers offer comes with the
// runtimes the backend pushes, not with the list. It follows the project from the day a
// repository is open, not from the day a page is shown: an install goes on, and its output is
// kept, whichever page is shown.

import type { ServerInfo } from "../generated/ServerInfo";
import type { ServerRuntimes } from "../generated/ServerRuntimes";
import { errorText } from "../lib/errorText";
import type { ServersAnswer } from "./backend";
import { type InstallProgress, appendLog } from "./installLog";
import { Resource } from "./resource";
import { session } from "./session.svelte";

/** What no server is known to do, older than anything the backend says. */
const NOTHING_KNOWN: ServerRuntimes = { serial: -1, runtimes: [], features: [] };

class ServerList {
  servers = $state.raw<ServerInfo[]>([]);
  /** What the servers of the changeset on view are doing: the backend's latest word. */
  runtimes = $state.raw<ServerRuntimes>(NOTHING_KNOWN);
  loading = $state(false);
  loadError = $state<string>();
  installs = $state.raw<Partial<Record<string, InstallProgress>>>({});
  /** The output of each language's install. */
  logs = $state.raw<Record<string, string[]>>({});
  /** Whether each language's install output is shown. */
  logOpen = $state.raw<Record<string, boolean>>({});
  private reads = new Resource<ServersAnswer>({
    busy: (on) => (this.loading = on),
    apply: (answer) => this.set(answer),
    failed: (error) => (this.loadError = errorText(error)),
  });

  constructor() {
    session.onConnect((backend) =>
      backend.onEvent((event) => {
        if (event.project !== backend.project) return;
        if (event.type === "serverInstallOutput") {
          this.logs = { ...this.logs, [event.language]: appendLog(this.logs[event.language] ?? [], event.line) };
          return;
        }
        if (event.type === "serverRuntimes") this.hear(event.runtimes);
        else if (event.type === "serversChanged") this.reload();
      }),
    );
    session.onOpen(() => {
      this.runtimes = NOTHING_KNOWN;
      this.reload();
    });
    session.onResync(() => {
      // A backend that restarted numbers what it says afresh.
      this.runtimes = NOTHING_KNOWN;
      if (session.repo !== undefined) this.reload();
    });
    session.onClose(() => {
      this.reads.cancel();
      this.servers = [];
      this.runtimes = NOTHING_KNOWN;
      this.loadError = undefined;
      this.installs = {};
      this.logs = {};
      this.logOpen = {};
    });
  }

  /** There is an open repository whose project can be asked. */
  get available(): boolean {
    return session.repo !== undefined && session.backend?.project !== undefined;
  }

  /** Reads the servers again; the latest read counts. */
  reload() {
    const backend = session.backend;
    if (backend) void this.reads.load(() => backend.listServers());
  }

  /** Takes the list an install, a new path or a restart answered with, which is as current as a read. */
  private replace(answer: ServersAnswer) {
    this.reads.cancel();
    this.set(answer);
  }

  private set({ servers, runtimes }: ServersAnswer) {
    this.servers = servers;
    this.hear(runtimes);
    this.loadError = undefined;
  }

  /** Takes what the backend says of the servers if it is newer than what is held. */
  private hear(runtimes: ServerRuntimes) {
    if (runtimes.serial > this.runtimes.serial) this.runtimes = runtimes;
  }

  /** Restarts the servers of `language` in the open project; resolves with why they did not start, if so. */
  async restart(language: string): Promise<string | undefined> {
    const backend = session.backend;
    if (!backend) return "No project is open";
    try {
      this.replace(await backend.restartServer(language));
      return undefined;
    } catch (error) {
      return errorText(error);
    }
  }

  async install(language: string) {
    const backend = session.backend;
    if (!backend) return;
    this.installs = { ...this.installs, [language]: { running: true } };
    this.logs = { ...this.logs, [language]: [] };
    this.logOpen = { ...this.logOpen, [language]: true };
    try {
      this.replace(await backend.installServer(language));
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
      this.replace(await backend.setServerPath(language, path));
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

export const serverList = new ServerList();
