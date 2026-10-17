// Which language server serves a file, as the backend decides it (by the file's name and, where a
// kind of file belongs to several projects, by the project's own files). Asked in batches, as
// the views come to need a file, and remembered; what is remembered is asked again when the servers
// change, so what shows meanwhile is the last answer.

import { SvelteMap } from "svelte/reactivity";
import type { Serving } from "../generated/Serving";
import { session } from "./session.svelte";

class ServedBy {
  private known = new SvelteMap<string, Serving>();
  private wanted = new Set<string>();
  private scheduled = false;
  /** Counts the times the project was left, so that an answer asked before is ignored. */
  private epoch = 0;

  constructor() {
    session.onConnect((backend) =>
      backend.onEvent((event) => {
        if (event.type === "serversChanged") this.refresh();
      }),
    );
    session.onResync(() => this.refresh());
    session.onClose(() => this.forget());
  }

  /** Which server serves `path`; `undefined` until the backend has said. */
  of(path: string): Serving | undefined {
    const serving = this.known.get(path);
    if (serving === undefined) this.want([path]);
    return serving;
  }

  /** The language of the server that serves `path`; `undefined` while unknown and when none does. */
  language(path: string): string | undefined {
    const serving = this.of(path);
    return serving?.type === "language" ? serving.language : undefined;
  }

  private want(paths: readonly string[]) {
    for (const path of paths) this.wanted.add(path);
    if (this.scheduled || !this.wanted.size) return;
    this.scheduled = true;
    queueMicrotask(() => void this.ask());
  }

  private async ask() {
    this.scheduled = false;
    const backend = session.backend;
    const paths = [...this.wanted];
    this.wanted.clear();
    if (!backend || !paths.length) return;
    const epoch = this.epoch;
    try {
      const serving = await backend.servedBy(paths);
      if (epoch !== this.epoch) return;
      paths.forEach((path, at) => this.known.set(path, serving[at]));
    } catch {
      // Asked again by the next view that needs the files.
    }
  }

  /** Asks again for everything known: the servers changed, and with them perhaps who serves what. */
  private refresh() {
    this.want([...this.known.keys()]);
  }

  private forget() {
    this.epoch++;
    this.wanted.clear();
    this.known.clear();
  }
}

export const servedBy = new ServedBy();
