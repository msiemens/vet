// The diagnostics of a changeset's files, asked in two phases: the target side's at once, with
// their origin pending, then a comparison with the base side's, which is slow to analyse and so
// waits for the file's server to finish its work and runs a few files at a time. Answers belong to
// an epoch; dropping them aborts what is underway and ignores what still arrives.

import type { Diagnostic } from "../generated/Diagnostic";
import type { Unavailable } from "../generated/Unavailable";
import { Gate } from "./gate.ts";
import { Latest } from "./resource.ts";

/** Files compared with the base at once. */
const COMPARED_AT_ONCE = 2;

/** Times a file is asked about because the server was at work on its own while it answered. */
const TRIES = 3;

/** A file's target-side diagnostics, each with its origin. */
export interface FileDiagnostics {
  diagnostics: Diagnostic[];
  /** The base side has answered (or failed to): the origins are as known as they will be. */
  compared: boolean;
  /** Why the origins are unknown, when the base side could not be analysed. */
  originUnavailable?: Unavailable;
}

export interface Answered {
  diagnostics: Diagnostic[];
  unavailable?: Unavailable;
  originUnavailable?: Unavailable;
  /** A server was at work on its own while it answered: the answer is not final, and asking again once it is done gives the rest. */
  incomplete?: Unavailable;
}

export interface BookHost {
  ask(revision: number, path: string, againstBase: boolean, signal: AbortSignal): Promise<Answered>;
  /** Resolves once the server of the file's language is not at work, or `signal` aborts. */
  untilIdle(path: string, signal: AbortSignal): Promise<void>;
  /** What the store keeps beside the diagnostics is dropped with them. */
  drop(): void;
  /** What `files` held when the revision advanced, for what shows until the new revision has answered. */
  hold(previous: ReadonlyMap<string, FileDiagnostics>): void;
}

export class DiagnosticsBook {
  private revision = -1;
  private epoch = new Latest();
  private abort = new AbortController();
  private pending = new Map<string, Promise<void>>();
  private comparing = new Gate(COMPARED_AT_ONCE);
  private listeners = new Set<(path: string) => void>();

  private readonly host: BookHost;
  /** The latest answer of each file asked about at the current revision. */
  private readonly files: Map<string, FileDiagnostics>;
  /** The files the backend could not answer for, and why. */
  private readonly missing: Map<string, Unavailable>;

  constructor(host: BookHost, files: Map<string, FileDiagnostics>, missing: Map<string, Unavailable>) {
    this.host = host;
    this.files = files;
    this.missing = missing;
  }

  /** The files of a new revision are asked afresh; `false` for a request of an older one. */
  enter(revision: number): boolean {
    if (revision < this.revision) return false;
    if (revision > this.revision) {
      this.revision = revision;
      const held = new Map(this.files);
      this.invalidate();
      this.host.hold(held);
    }
    return true;
  }

  /** A repository was opened: its revisions are numbered afresh. */
  reset() {
    this.revision = -1;
    this.invalidate();
  }

  /** What the servers answered may differ now: everything underway is aborted and nothing is kept. */
  invalidate() {
    this.epoch.cancel();
    this.abort.abort();
    this.abort = new AbortController();
    this.pending.clear();
    this.files.clear();
    this.missing.clear();
    this.host.hold(new Map());
    this.host.drop();
  }

  /** Calls `listener` with the path of a file whose diagnostics were replaced by ones with their origins. */
  onOrigins(listener: (path: string) => void): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  /** The target-side diagnostics of a file, asked once per revision; empty if unavailable or stale. */
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

  private async fetch(revision: number, path: string): Promise<void> {
    const epoch = this.epoch.mark();
    const { signal } = this.abort;
    try {
      const r = await this.host.ask(revision, path, false, signal);
      if (this.epoch.movedSince(epoch)) return;
      this.take(path, r, false);
      if (!r.unavailable) void this.compare(revision, path, epoch, signal, r.incomplete !== undefined);
    } catch {
      if (!this.epoch.movedSince(epoch)) this.pending.delete(path); // stale: the next revision asks again
    }
  }

  /** Asks the target side again while its server keeps working, which the answers say; false if the file has none now. */
  private async refresh(revision: number, path: string, epoch: number, signal: AbortSignal): Promise<boolean> {
    for (let tries = 1; tries < TRIES; tries++) {
      await this.host.untilIdle(path, signal);
      if (signal.aborted || this.epoch.movedSince(epoch)) return false;
      const r = await this.host.ask(revision, path, false, signal);
      if (this.epoch.movedSince(epoch)) return false;
      this.take(path, r, false);
      if (r.unavailable) return false;
      if (!r.incomplete) break;
    }
    return true;
  }

  private async compare(revision: number, path: string, epoch: number, signal: AbortSignal, partial: boolean) {
    let failure: Unavailable;
    try {
      if (partial && !(await this.refresh(revision, path, epoch, signal))) return;
      for (let tries = 1; ; tries++) {
        await this.host.untilIdle(path, signal);
        const r = await this.comparing.run(() =>
          signal.aborted ? Promise.resolve(undefined) : this.host.ask(revision, path, true, signal),
        );
        if (!r || this.epoch.movedSince(epoch)) return;
        // An answer given while the server was at work may lack what it had not got to yet.
        if (r.incomplete && tries < TRIES) continue;
        if (r.unavailable) {
          failure = r.unavailable;
          break;
        }
        this.take(path, r, true);
        this.announce(path);
        return;
      }
    } catch (error) {
      failure = {
        kind: "queryFailed",
        label: "Comparison failed",
        message: error instanceof Error ? error.message : String(error),
        language: null,
      };
    }
    if (this.epoch.movedSince(epoch)) return;
    this.unknown(path, failure);
    this.announce(path);
  }

  /** The file's diagnostics stay as they are, their origins unknown for `why`. */
  private unknown(path: string, why: Unavailable) {
    const held = this.files.get(path);
    if (!held) return;
    this.files.set(path, {
      diagnostics: held.diagnostics.map((d) => ({ ...d, origin: "unknown" })),
      compared: true,
      originUnavailable: why,
    });
  }

  private announce(path: string) {
    for (const listener of this.listeners) listener(path);
  }

  private take(path: string, r: Answered, compared: boolean) {
    if (r.unavailable) {
      this.missing.set(path, r.unavailable);
      return;
    }
    this.missing.delete(path);
    this.files.set(path, { diagnostics: r.diagnostics, compared, originUnavailable: r.originUnavailable });
  }
}
