// The semantic layer's answers for the views: hover, definition and diagnostics, with the
// diagnostics count and the "no language server" notes the footer shows (SPEC §7.1, I6). Every
// request carries the revision of the view that asks; a stale one is rejected by the backend and
// dropped here, as the views are rebuilt when the new revision arrives.

import { SvelteMap } from "svelte/reactivity";
import type { Diagnostic } from "../generated/Diagnostic";
import type { SemanticLocation } from "../generated/SemanticLocation";
import type { Side } from "../generated/Side";
import type { Unavailable } from "../generated/Unavailable";
import { isAbort } from "../lib/abort";
import type { Backend, HoverAnswer } from "./backend";
import { changeset } from "./changeset.svelte";
import { commands } from "./commands";
import { DefinitionProbe } from "./definitionProbe";
import { Gate } from "./gate";
import { InFlight } from "./inFlight";
import { progress } from "./progress.svelte";
import { type SemanticSummary, summarize } from "./semanticSummary";
import { serverList } from "./serverList.svelte";
import { hasServer, serverLine } from "./serverStatus";
import { session } from "./session.svelte";

const NOTHING: HoverAnswer = { text: "", code: [] };

/** Files compared with the base at once. */
const COMPARED_AT_ONCE = 2;

/** A file's target-side diagnostics, each with its origin. */
export interface FileDiagnostics {
  diagnostics: Diagnostic[];
  /** The base side has answered (or failed to): the origins are as known as they will be. */
  compared: boolean;
  /** Why the origins are unknown, when the base side could not be analysed. */
  originUnavailable?: Unavailable;
}

class SemanticStore {
  private revision = -1;
  /** Bumped when the cache is dropped, so an answer to an earlier question is not stored. */
  private epoch = 0;
  private pending = new Map<string, Promise<void>>();
  private comparing = new Gate(COMPARED_AT_ONCE);
  private originListeners = new Set<(path: string) => void>();
  /** The latest diagnostics of each file asked about at `revision`, with the origins once they are known. */
  files = new SvelteMap<string, FileDiagnostics>();
  /** What `files` held when the revision last advanced, for what shows until the new revision has answered. */
  previous = $state.raw<ReadonlyMap<string, FileDiagnostics>>(new Map());
  /** Counts the times the answers were dropped; what was asked of the servers is asked again when it changes. */
  generation = $state(0);
  private hovering = new InFlight<HoverAnswer>();
  private probe = new DefinitionProbe((key, signal) => this.lookUp(key, signal));
  /** The number of diagnostics to tell the viewer about, per file asked about at `revision`. */
  private counts = new SvelteMap<string, number>();
  /** The files the backend could not answer for, and why. */
  private missing = new SvelteMap<string, Unavailable>();

  summary(path?: string): SemanticSummary {
    return summarize(this.counts, this.missing, path);
  }

  /** The files of a new revision are asked afresh; `false` for a request of an older one. */
  private enter(revision: number): boolean {
    if (revision < this.revision) return false;
    if (revision > this.revision) {
      this.revision = revision;
      const held = new Map(this.files);
      this.invalidate();
      this.previous = held;
    }
    return true;
  }

  /** A repository was opened: its revisions are numbered afresh. */
  reset() {
    this.revision = -1;
    this.invalidate();
    this.previous = new Map();
  }

  /** The reason the backend gave for having no diagnostics of `path`, if it did. */
  unavailableOf(path: string): Unavailable | undefined {
    return this.missing.get(path);
  }

  /** What the servers answered may differ now (a server was installed or its path changed). */
  invalidate() {
    this.epoch++;
    this.generation++;
    this.pending.clear();
    this.hovering.clear();
    this.probe.clear();
    this.counts.clear();
    this.missing.clear();
    this.files.clear();
  }

  /** Remembers that the file has no answers. A server lacking one operation still serves the file. */
  private note(revision: number, path: string, unavailable: Unavailable | undefined) {
    if (unavailable && unavailable.kind !== "notOffered" && this.enter(revision)) this.missing.set(path, unavailable);
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
    if (!backend || !hasServer(path, serverList.servers)) return NOTHING;
    try {
      return await this.hovering.share(
        JSON.stringify([revision, side, path, line, col]),
        async (shared) => {
          const r = await backend.hover(revision, side, path, line, col, shared);
          this.note(revision, path, r.unavailable);
          return {
            ...r,
            server: serverLine(path, serverList.servers, progress.tasks, this.summary(path)),
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
      if (r.unavailable) session.notice = r.unavailable.message;
      else if (!r.locations.length) session.notice = "No definition found";
      else return r.locations;
    } catch (error) {
      // Only the changeset moving on aborts a definition: the view that asked is about to be replaced.
      if (isAbort(error)) session.notice = "Changes arrived — try again";
      else changeset.reportFailure(error, revision);
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
    if (!session.backend || !hasServer(path, serverList.servers)) return false;
    if (!this.enter(revision)) return false;
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
   * The target-side diagnostics of a file, asked once per revision; empty if unavailable or stale. They
   * come at once with their origin pending, and a comparison with the base follows, whose answer
   * replaces them (`files`) and is announced to the listeners.
   */
  async diagnostics(revision: number, path: string): Promise<Diagnostic[]> {
    if (!this.enter(revision)) return [];
    let answer = this.pending.get(path);
    if (!answer) {
      answer = this.fetch(revision, path);
      this.pending.set(path, answer);
    }
    await answer;
    return this.files.get(path)?.diagnostics ?? [];
  }

  /** Calls `listener` with the path of a file whose diagnostics were replaced by ones with their origins. */
  onOrigins(listener: (path: string) => void): () => void {
    this.originListeners.add(listener);
    return () => this.originListeners.delete(listener);
  }

  private async fetch(revision: number, path: string): Promise<void> {
    const backend = session.backend;
    const epoch = this.epoch;
    if (!backend) return;
    try {
      const r = await backend.diagnostics(revision, path, false);
      if (epoch !== this.epoch) return;
      this.take(path, r, false);
      if (!r.unavailable) void this.compare(backend, revision, path, epoch);
    } catch {
      if (epoch === this.epoch) this.pending.delete(path); // stale: the next revision asks again
    }
  }

  /** Asks for the file's diagnostics with their origins, a few files at a time: the base side is slow to analyse. */
  private async compare(backend: Backend, revision: number, path: string, epoch: number) {
    try {
      const r = await this.comparing.run(() =>
        epoch === this.epoch ? backend.diagnostics(revision, path, true) : Promise.resolve(undefined),
      );
      if (!r || epoch !== this.epoch || r.unavailable) return;
      this.take(path, r, true);
      for (const listener of this.originListeners) listener(path);
    } catch {
      // The diagnostics stay as they are, with their origin pending.
    }
  }

  private take(
    path: string,
    r: { diagnostics: Diagnostic[]; count: number; unavailable?: Unavailable; originUnavailable?: Unavailable },
    compared: boolean,
  ) {
    if (r.unavailable) {
      this.missing.set(path, r.unavailable);
      return;
    }
    this.missing.delete(path);
    this.counts.set(path, r.count);
    this.files.set(path, { diagnostics: r.diagnostics, compared, originUnavailable: r.originUnavailable });
  }
}

export const semantic = new SemanticStore();
