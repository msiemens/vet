// The semantic layer's answers for the views: hover, definition, and the diagnostics the backend
// pushes for the changeset's files, with the "no language server" notes the footer shows (SPEC
// §6.1, I3). The backend analyses the changed files itself while the review screen shows the
// changeset, the files with an open tab first, and pushes each answer (A13), compared with the
// base; this applies them and answers what the views ask of the pushed state.

import { untrack } from "svelte";
import { SvelteMap } from "svelte/reactivity";
import type { Diagnostic } from "../generated/Diagnostic";
import type { SemanticLocation } from "../generated/SemanticLocation";
import type { Side } from "../generated/Side";
import type { Unavailable } from "../generated/Unavailable";
import { isAbort } from "../lib/abort";
import { STALE_NOTICE } from "../lib/notice";
import type { HoverAnswer } from "./backend";
import { commands } from "./commands";
import { DefinitionProbe } from "./definitionProbe";
import { InFlight } from "./inFlight";
import { notices } from "./notices.svelte";
import { opened } from "./opened.svelte";
import { type SemanticSummary, summarize } from "./semanticSummary";
import { servedBy } from "./servedBy.svelte";
import { serverList } from "./serverList.svelte";
import { hasServer, serverLine } from "./serverStatus";
import { session } from "./session.svelte";

const NOTHING: HoverAnswer = { text: "", code: [] };

/** A file's target-side diagnostics, each with its origin, as the backend pushed them. */
export interface FileDiagnostics {
  diagnostics: Diagnostic[];
  /** The base side has been compared: the origins are as known as they will be. */
  compared: boolean;
  /** Why the origins are unknown, when the base side could not be analysed. */
  originUnavailable?: Unavailable;
}

class SemanticStore {
  /** The latest diagnostics of each file of the changeset on view, with the origins once they are known. */
  files = new SvelteMap<string, FileDiagnostics>();
  /** What `files` held when the revision last advanced, for what shows until the new revision has answered. */
  previous = $state.raw<ReadonlyMap<string, FileDiagnostics>>(new Map());
  private hovering = new InFlight<HoverAnswer>();
  private probe = new DefinitionProbe((key, signal) => this.lookUp(key, signal));
  /** The files the backend could not answer for, and why. */
  private missing = new SvelteMap<string, Unavailable>();
  /** The listeners told when a file's diagnostics were replaced. */
  private listeners = new Set<(path: string) => void>();
  /** The changeset revision the answers are for; an answer for another is dropped. */
  private revision?: number;

  constructor() {
    session.onConnect((backend) =>
      backend.onEvent((event) => {
        if (event.type !== "fileDiagnostics") return;
        this.apply(event.revision, event.path, event);
      }),
    );
    $effect.root(() => {
      $effect(() => {
        const revision = opened.session?.layout?.shown.revision;
        untrack(() => this.enter(revision));
      });
    });
  }

  summary(path?: string): SemanticSummary {
    return summarize(this.files, this.missing, path);
  }

  /** The changeset's revision the answers are for; a new one moves what is held to `previous`. */
  private enter(revision: number | undefined) {
    if (revision === this.revision) return;
    this.previous = revision !== undefined && this.revision !== undefined ? new Map(this.files) : new Map();
    this.revision = revision;
    this.drop();
  }

  /** A repository was opened: its revisions are numbered afresh. */
  reset() {
    this.revision = undefined;
    this.previous = new Map();
    this.drop();
  }

  /** What the servers answered may differ now (a server was installed or its path changed). */
  invalidate() {
    this.drop();
  }

  private drop() {
    this.files.clear();
    this.missing.clear();
    this.hovering.clear();
    this.probe.clear();
  }

  /** The reason the backend gave for having no diagnostics of `path`, if it did. */
  unavailableOf(path: string): Unavailable | undefined {
    return this.missing.get(path);
  }

  /** Remembers that the file has no answers. A server lacking one operation still serves the file. */
  private note(revision: number, path: string, unavailable: Unavailable | undefined) {
    if (unavailable && unavailable.kind !== "notOffered" && revision === this.revision)
      this.missing.set(path, unavailable);
  }

  /**
   * Markdown; the text is empty if the server has nothing to say, the request is stale or `signal`
   * aborted it, and with no server for the file's type nothing is asked. The same position asked again while its answer is underway (an editor re-fires its
   * hover when it updates) joins that request, which is cancelled once no caller wants it.
   */
  async hover(
    revision: number,
    side: Side,
    path: string,
    line: number,
    col: number,
    signal?: AbortSignal,
  ): Promise<HoverAnswer> {
    const backend = session.backend;
    if (!backend || !hasServer(servedBy.of(path), serverList.servers)) return NOTHING;
    try {
      return await this.hovering.share(
        JSON.stringify([revision, side, path, line, col]),
        async (shared) => {
          const r = await backend.hover(revision, side, path, line, col, shared);
          this.note(revision, path, r.unavailable);
          return {
            ...r,
            server: serverLine(path, servedBy.of(path), serverList.servers, serverList.runtimes, this.summary(path)),
            definitionKey: commands.binding("review.goToDefinition")?.label,
          };
        },
        signal,
      );
    } catch {
      return NOTHING;
    }
  }

  /** The definitions at the position, in `side`'s snapshot; `undefined` after telling the viewer why there are none. */
  async definition(
    revision: number,
    side: Side,
    path: string,
    line: number,
    col: number,
  ): Promise<SemanticLocation[] | undefined> {
    const backend = session.backend;
    if (!backend) return undefined;
    try {
      const r = await backend.definition(revision, side, path, line, col);
      this.note(revision, path, r.unavailable);
      if (r.unavailable) notices.show(r.unavailable.message);
      else if (!r.locations.length) notices.show("No definition found");
      else return r.locations;
    } catch (error) {
      // Only the changeset moving on aborts a definition: the view that asked is about to be replaced.
      if (isAbort(error)) notices.show(STALE_NOTICE);
      else opened.session?.reportFailure(error, revision);
    }
    return undefined;
  }

  /**
   * Whether the position has a definition, asked once per position and without telling the viewer
   * anything; `false` with no server for the file's type, and on a failure (which is asked again).
   */
  async hasDefinition(
    revision: number,
    side: Side,
    path: string,
    line: number,
    col: number,
    signal?: AbortSignal,
  ): Promise<boolean> {
    if (!session.backend || !hasServer(servedBy.of(path), serverList.servers)) return false;
    if (revision !== this.revision) return false;
    return this.probe.has(JSON.stringify([revision, side, path, line, col]), signal);
  }

  private async lookUp(key: string, signal: AbortSignal): Promise<boolean> {
    const [revision, side, path, line, col] = JSON.parse(key) as [number, Side, string, number, number];
    const r = await session.backend?.definition(revision, side, path, line, col, signal);
    if (!r) return false;
    this.note(revision, path, r.unavailable);
    // An unavailable server (still starting, say) may answer later, so this is no answer to keep;
    // one that does not offer definitions never will.
    if (r.unavailable && r.unavailable.kind !== "notOffered") throw new Error(r.unavailable.message);
    return r.locations.length > 0;
  }

  /**
   * The target-side diagnostics of a file as the backend pushed them, or empty while it has not
   * answered for it (or the answer is of another revision).
   */
  diagnostics(revision: number, path: string): Promise<Diagnostic[]> {
    if (revision !== this.revision) return Promise.resolve([]);
    return Promise.resolve(this.files.get(path)?.diagnostics ?? []);
  }

  /** Calls `listener` with the path of a file whose diagnostics were replaced. */
  onOrigins(listener: (path: string) => void): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  /** The backend analysed one file: apply what it pushed and tell the listeners. */
  private apply(
    revision: number,
    path: string,
    answer: {
      diagnostics: Diagnostic[];
      compared: boolean;
      unavailable: Unavailable | null;
      originUnavailable: Unavailable | null;
    },
  ) {
    if (revision !== this.revision) return;
    if (answer.unavailable) {
      this.missing.set(path, answer.unavailable);
      this.files.delete(path);
    } else {
      this.missing.delete(path);
      this.files.set(path, {
        diagnostics: answer.diagnostics,
        compared: answer.compared,
        originUnavailable: answer.originUnavailable ?? undefined,
      });
    }
    for (const listener of this.listeners) listener(path);
  }
}

export const semantic = new SemanticStore();
