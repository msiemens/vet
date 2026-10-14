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
import { STALE_NOTICE } from "../lib/notice";
import type { HoverAnswer } from "./backend";
import { changeset } from "./changeset.svelte";
import { commands } from "./commands";
import { DefinitionProbe } from "./definitionProbe";
import { DiagnosticsBook, type FileDiagnostics } from "./diagnosticsBook";
import { InFlight } from "./inFlight";
import { notices } from "./notices.svelte";
import { progress } from "./progress.svelte";
import { type SemanticSummary, summarize } from "./semanticSummary";
import { serverList } from "./serverList.svelte";
import { atWork, hasServer, serverLine } from "./serverStatus";
import { session } from "./session.svelte";

const NOTHING: HoverAnswer = { text: "", code: [] };

class SemanticStore {
  /** The latest diagnostics of each file asked about at the current revision, with the origins once they are known. */
  files = new SvelteMap<string, FileDiagnostics>();
  /** What `files` held when the revision last advanced, for what shows until the new revision has answered. */
  previous = $state.raw<ReadonlyMap<string, FileDiagnostics>>(new Map());
  /** Counts the times the answers were dropped; what was asked of the servers is asked again when it changes. */
  generation = $state(0);
  private hovering = new InFlight<HoverAnswer>();
  private probe = new DefinitionProbe((key, signal) => this.lookUp(key, signal));
  /** The files the backend could not answer for, and why. */
  private missing = new SvelteMap<string, Unavailable>();
  private book = new DiagnosticsBook(
    {
      ask: (revision, path, againstBase, signal) => this.askDiagnostics(revision, path, againstBase, signal),
      untilIdle: (path, signal) => this.untilIdle(path, signal),
      drop: () => {
        this.generation++;
        this.hovering.clear();
        this.probe.clear();
      },
      hold: (held) => (this.previous = held),
    },
    this.files,
    this.missing,
  );

  summary(path?: string): SemanticSummary {
    return summarize(this.files, this.missing, path);
  }

  private enter(revision: number): boolean {
    return this.book.enter(revision);
  }

  /** A repository was opened: its revisions are numbered afresh. */
  reset() {
    this.book.reset();
  }

  /** The reason the backend gave for having no diagnostics of `path`, if it did. */
  unavailableOf(path: string): Unavailable | undefined {
    return this.missing.get(path);
  }

  /** What the servers answered may differ now (a server was installed or its path changed). */
  invalidate() {
    this.book.invalidate();
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
      if (r.unavailable) notices.show(r.unavailable.message);
      else if (!r.locations.length) notices.show("No definition found");
      else return r.locations;
    } catch (error) {
      // Only the changeset moving on aborts a definition: the view that asked is about to be replaced.
      if (isAbort(error)) notices.show(STALE_NOTICE);
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
   * replaces them (`files`) and is announced to the `onOrigins` listeners.
   */
  diagnostics(revision: number, path: string): Promise<Diagnostic[]> {
    return this.book.diagnostics(revision, path);
  }

  /** Calls `listener` with the path of a file whose diagnostics were replaced by ones with their origins. */
  onOrigins(listener: (path: string) => void): () => void {
    return this.book.onOrigins(listener);
  }

  private askDiagnostics(revision: number, path: string, againstBase: boolean, signal: AbortSignal) {
    const backend = session.backend;
    if (!backend) return Promise.reject(new Error("no backend"));
    return backend.diagnostics(revision, path, againstBase, signal);
  }

  /** Resolves once the server of the file's language is not starting or indexing. */
  private async untilIdle(path: string, signal: AbortSignal): Promise<void> {
    const stops: (() => void)[] = [];
    await new Promise<void>((resolve) => {
      if (signal.aborted) {
        resolve();
        return;
      }
      signal.addEventListener("abort", () => resolve(), { once: true });
      stops.push(
        $effect.root(() => {
          $effect(() => {
            if (!atWork(path, serverList.servers, progress.tasks)) resolve();
          });
        }),
      );
    });
    for (const stop of stops) stop();
  }
}

export const semantic = new SemanticStore();
