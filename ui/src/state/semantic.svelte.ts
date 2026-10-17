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
import type { HoverText } from "./backend";
import { DefinitionProbe } from "./definitionProbe";
import { InFlight } from "./inFlight";
import { opened } from "./opened.svelte";
import { type SemanticSummary, summarize } from "./semanticSummary";
import { servedBy } from "./servedBy.svelte";
import { serverList } from "./serverList.svelte";
import { hasServer } from "./serverStatus";
import { session } from "./session.svelte";

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
  private hovering = new InFlight<HoverText>();
  private probe = new DefinitionProbe((key, signal) => this.lookUp(key, signal));
  /** The files the backend could not answer for, and why. */
  private missing = new SvelteMap<string, Unavailable>();
  /** The listeners told when a file's diagnostics were replaced. */
  private listeners = new Set<(path: string | undefined) => void>();
  /** The session the answers are of, by identity. */
  private owner: typeof opened.session;
  /** The changeset revision the answers are for; an answer for another is dropped. */
  private revision?: number;

  constructor() {
    session.onConnect((backend) =>
      backend.onEvent((event) => {
        if (event.type === "fileDiagnostics") this.apply(event.revision, event.path, event);
        else if (event.type === "serversChanged") this.invalidate();
      }),
    );
    $effect.root(() => {
      $effect(() => {
        const owner = opened.session;
        const revision = owner?.layout?.shown.revision;
        untrack(() => this.enter(owner, revision));
      });
    });
  }

  summary(path?: string): SemanticSummary {
    return summarize(this.files, this.missing, path);
  }

  /**
   * The session on view and its revision the answers are for: a newer revision of the same session
   * moves what is held to `previous`, another session starts over.
   */
  private enter(owner: typeof opened.session, revision: number | undefined) {
    if (owner === this.owner && revision === this.revision) return;
    const newer = owner === this.owner && revision !== undefined && this.revision !== undefined;
    this.previous = newer ? new Map(this.files) : new Map();
    this.owner = owner;
    this.revision = revision;
    this.drop();
  }

  /**
   * What the servers answered may differ now (a server was installed or its path changed): every
   * answer goes, and the listeners hear of every file.
   */
  private invalidate() {
    this.drop();
    for (const listener of this.listeners) listener(undefined);
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
   * The server's Markdown, or `undefined` if nothing was asked (no server for the file's type), the
   * request failed or `signal` aborted it. The same position asked again while its answer is underway
   * (an editor re-fires its hover when it updates) joins that request, which is cancelled once no
   * caller wants it.
   */
  async hover(
    revision: number,
    side: Side,
    path: string,
    line: number,
    col: number,
    signal?: AbortSignal,
  ): Promise<HoverText | undefined> {
    const backend = session.backend;
    if (!backend || !hasServer(servedBy.of(path), serverList.servers)) return undefined;
    try {
      return await this.hovering.share(
        JSON.stringify([revision, side, path, line, col]),
        async (shared) => {
          const r = await backend.hover(revision, side, path, line, col, shared);
          this.note(revision, path, r.unavailable);
          return r;
        },
        signal,
      );
    } catch {
      return undefined;
    }
  }

  /**
   * The definitions at the position, in `side`'s snapshot, as the backend answered; `undefined`
   * without a backend. Rejects as the request does.
   */
  async definition(
    revision: number,
    side: Side,
    path: string,
    line: number,
    col: number,
  ): Promise<{ locations: SemanticLocation[]; unavailable?: Unavailable } | undefined> {
    const backend = session.backend;
    if (!backend) return undefined;
    const r = await backend.definition(revision, side, path, line, col);
    this.note(revision, path, r.unavailable);
    return r;
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

  /**
   * Calls `listener` with the path of a file whose diagnostics were replaced, or with none when
   * every file's were dropped.
   */
  onOrigins(listener: (path: string | undefined) => void): () => void {
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
