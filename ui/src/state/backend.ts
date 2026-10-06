// Typed requests over the connection. The only module besides `connection.ts` that
// knows the wire protocol, so a protocol change lands here.

import type { ApprovalFile } from "../generated/ApprovalFile";
import type { Base } from "../generated/Base";
import type { BaseChoice } from "../generated/BaseChoice";
import type { BlobContent } from "../generated/BlobContent";
import type { BranchInfo } from "../generated/BranchInfo";
import type { ChangedSymbols } from "../generated/ChangedSymbols";
import type { CommitInfo } from "../generated/CommitInfo";
import type { Diagnostic } from "../generated/Diagnostic";
import type { DirEntry } from "../generated/DirEntry";
import type { Event as ServerEvent } from "../generated/Event";
import type { Impact } from "../generated/Impact";
import type { Manifest } from "../generated/Manifest";
import type { NonTextKind } from "../generated/NonTextKind";
import type { Reachability } from "../generated/Reachability";
import type { RemovalImpact } from "../generated/RemovalImpact";
import type { Response } from "../generated/Response";
import type { ReviewFilter } from "../generated/ReviewFilter";
import type { ReviewUpdate } from "../generated/ReviewUpdate";
import type { RunTarget } from "../generated/RunTarget";
import type { RunTargetEntry } from "../generated/RunTargetEntry";
import type { SemanticLocation } from "../generated/SemanticLocation";
import type { ServerInfo } from "../generated/ServerInfo";
import type { Side } from "../generated/Side";
import type { SymbolId } from "../generated/SymbolId";
import type { TextContent } from "../generated/TextContent";
import type { Thread } from "../generated/Thread";
import type { ThreadStatus } from "../generated/ThreadStatus";
import type { ThreadTarget } from "../generated/ThreadTarget";
import type { Unavailable } from "../generated/Unavailable";
import { Connection } from "./connection";

export type FileContents = Extract<Response, { type: "fileContents" }>;
export type RunNeedsApproval = Extract<Response, { type: "runNeedsApproval" }>;
export type RunStarted = Extract<Response, { type: "runStarted" }>;
export type RunAnswer = RunNeedsApproval | RunStarted;

export interface BaseCandidates {
  branches: BranchInfo[];
  /** Newest first. */
  commits: CommitInfo[];
}

function expect<T extends Response["type"]>(response: Response, type: T): Extract<Response, { type: T }> {
  if (response.type !== type) throw new Error("unexpected response");
  return response as Extract<Response, { type: T }>;
}

/** The `thread` response carries the thread's fields beside its tag. */
const asThread = (response: Extract<Response, { type: "thread" }>): Thread => response;

export class Backend {
  /** The open session, if any; one at a time. */
  session?: number;

  private constructor(
    private conn: Connection,
    /** The repository the host (or the URL hash) named, if any. */
    readonly initialRepo: string | null,
  ) {}

  static async connect(): Promise<Backend> {
    const conn = await Connection.open();
    return new Backend(conn, conn.repo);
  }

  /** Opens a session on `repo`; resolves with the repository root the backend settled on. */
  async open(repo: string): Promise<string> {
    const opened = expect(await this.conn.request({ type: "openSession", repo }), "session");
    this.session = opened.session;
    // The backend resolves a subfolder to its repository root and reports it.
    return (opened as { root?: string }).root ?? repo;
  }

  async close(): Promise<void> {
    const session = this.session;
    this.session = undefined;
    if (session !== undefined) await this.conn.request({ type: "closeSession", session });
  }

  private get sid(): number {
    if (this.session === undefined) throw new Error("no repository is open");
    return this.session;
  }

  onEvent(listener: (event: ServerEvent) => void): () => void {
    return this.conn.onEvent(listener);
  }

  async manifest(): Promise<Manifest> {
    return expect(await this.conn.request({ type: "manifest", session: this.sid }), "manifest");
  }

  fileContents(path: string): Promise<FileContents> {
    return this.conn.request({ type: "fileContents", session: this.sid, path }).then((r) => expect(r, "fileContents"));
  }

  /** The direct children of a directory of the target snapshot (`""` is the root), as of `revision`. */
  async listDirectory(path: string): Promise<{ revision: number; entries: DirEntry[] }> {
    const { revision, entries } = expect(
      await this.conn.request({ type: "listDirectory", session: this.sid, path }),
      "directory",
    );
    return { revision, entries };
  }

  /** The target-side text of any path of the target snapshot, changed or not. */
  async fileText(path: string): Promise<{ revision: number; content: TextContent }> {
    const { revision, content } = expect(
      await this.conn.request({ type: "fileText", session: this.sid, path }),
      "fileText",
    );
    return { revision, content };
  }

  /** One side of a non-text item's file, as of `revision`. */
  async itemBlob(revision: number, path: string, kind: NonTextKind, side: Side): Promise<BlobContent> {
    const request = { type: "itemBlob", session: this.sid, revision, path, kind, side } as const;
    return expect(await this.conn.request(request), "itemBlob").content;
  }

  /** `revision` is the manifest's; the backend rejects a stale one. */
  async setSeen(revision: number, path: string, hunks: number[], seen: boolean): Promise<ReviewUpdate> {
    return expect(
      await this.conn.request({ type: "setSeen", session: this.sid, revision, path, hunks, seen }),
      "review",
    );
  }

  async acknowledge(revision: number, path: string, kind: NonTextKind, acknowledged: boolean): Promise<ReviewUpdate> {
    return expect(
      await this.conn.request({ type: "acknowledge", session: this.sid, revision, path, kind, acknowledged }),
      "review",
    );
  }

  /** Compares against another base; every subscriber, this one included, then gets `snapshotSettled`. */
  async setBase(base: BaseChoice): Promise<Base> {
    return expect(await this.conn.request({ type: "setBase", session: this.sid, base }), "base");
  }

  /** Replaces the review filter; every subscriber, this one included, then gets `snapshotSettled`. Rejects with the backend's message for an invalid glob. */
  async setFilter(filter: ReviewFilter): Promise<void> {
    expect(await this.conn.request({ type: "setFilter", session: this.sid, filter }), "done");
  }

  /** The branches and recent commits a base can be picked from. */
  async baseCandidates(): Promise<BaseCandidates> {
    const { branches, commits } = expect(
      await this.conn.request({ type: "listBaseCandidates", session: this.sid }),
      "baseCandidates",
    );
    return { branches, commits };
  }

  /**
   * Hover at a 0-based line and UTF-16 column of `side`'s text at `revision`, the manifest's (the
   * backend rejects a stale one). `text` is empty if the server has nothing to say there.
   */
  async hover(
    revision: number,
    side: Side,
    path: string,
    line: number,
    col: number,
  ): Promise<{ text: string; unavailable?: Unavailable }> {
    const r = expect(
      await this.conn.request({ type: "hover", session: this.sid, revision, side, path, line, col }),
      "hover",
    );
    return { text: r.text ?? "", unavailable: r.unavailable ?? undefined };
  }

  /** Where the symbol at the position is defined, in `side`'s snapshot; otherwise like `hover`. */
  async definition(
    revision: number,
    side: Side,
    path: string,
    line: number,
    col: number,
  ): Promise<{ locations: SemanticLocation[]; unavailable?: Unavailable }> {
    const r = expect(
      await this.conn.request({ type: "definition", session: this.sid, revision, side, path, line, col }),
      "definition",
    );
    return { locations: r.locations, unavailable: r.unavailable ?? undefined };
  }

  /** The diagnostics of a file of the target side; the language server may take seconds on a cold start. */
  async diagnostics(revision: number, path: string): Promise<{ diagnostics: Diagnostic[]; unavailable?: Unavailable }> {
    const r = expect(
      await this.conn.request({ type: "diagnostics", session: this.sid, revision, path }),
      "diagnostics",
    );
    return { diagnostics: r.diagnostics, unavailable: r.unavailable ?? undefined };
  }

  /** Each language's server as it resolves for the open repository. */
  async listServers(): Promise<ServerInfo[]> {
    return expect(await this.conn.request({ type: "listServers", session: this.sid }), "servers").servers;
  }

  /** Installs the pinned server; output arrives as `serverInstallOutput` events. Rejects with the tool's output tail. */
  async installServer(language: string): Promise<ServerInfo[]> {
    return expect(await this.conn.request({ type: "installServer", session: this.sid, language }), "servers").servers;
  }

  /** `null` removes the custom path: automatic resolution again. The path is not checked; `listServers` says whether it works. */
  async setServerPath(language: string, path: string | null): Promise<ServerInfo[]> {
    return expect(await this.conn.request({ type: "setServerPath", session: this.sid, language, path }), "servers")
      .servers;
  }

  /** Discovered from the target snapshot's manifests, with the stored edits applied. */
  async runTargets(): Promise<RunTargetEntry[]> {
    return expect(await this.conn.request({ type: "listRunTargets", session: this.sid }), "runTargets").targets;
  }

  async saveRunTarget(target: RunTarget): Promise<void> {
    expect(await this.conn.request({ type: "saveRunTarget", session: this.sid, target }), "done");
  }

  /** Drops the stored edit of a discovered target, or deletes a custom one. */
  async resetRunTarget(id: string): Promise<void> {
    expect(await this.conn.request({ type: "resetRunTarget", session: this.sid, id }), "done");
  }

  /** Runs a target; without `approvedHash` the answer may ask for approval first. */
  async run(targetId: string, approvedHash?: string): Promise<RunAnswer> {
    const response = await this.conn.request({ type: "run", session: this.sid, targetId, approvedHash });
    if (response.type === "runNeedsApproval") return response;
    return expect(response, "runStarted");
  }

  /** The files behind a pending approval, with their text. */
  async runApprovalDiff(hash: string): Promise<ApprovalFile[]> {
    return expect(await this.conn.request({ type: "runApprovalDiff", session: this.sid, hash }), "runApprovalDiff")
      .files;
  }

  async stopRun(runId: number): Promise<void> {
    expect(await this.conn.request({ type: "stopRun", session: this.sid, runId }), "done");
  }

  /** The symbols the changeset touches, as of `revision` (the manifest's; a stale one is rejected). */
  async changedSymbols(revision: number): Promise<ChangedSymbols> {
    return expect(await this.conn.request({ type: "changedSymbols", session: this.sid, revision }), "changedSymbols");
  }

  /** References and callers of an added or modified changed symbol; may take a while on a cold server. */
  async impact(revision: number, symbol: SymbolId): Promise<Impact> {
    return expect(await this.conn.request({ type: "impact", session: this.sid, revision, symbol }), "impact");
  }

  /** What became of the references to a removed changed symbol. */
  async removalImpact(revision: number, symbol: SymbolId): Promise<RemovalImpact> {
    return expect(
      await this.conn.request({ type: "removalImpact", session: this.sid, revision, symbol }),
      "removalImpact",
    );
  }

  /** Whether an added or modified changed symbol is reached from an entry point. */
  async reachability(revision: number, symbol: SymbolId): Promise<Reachability> {
    return expect(
      await this.conn.request({ type: "reachability", session: this.sid, revision, symbol }),
      "reachability",
    );
  }

  /** Every thread with its anchor resolved at `revision`, the current one, in creation order. */
  async threads(): Promise<{ revision: number; threads: Thread[] }> {
    const { revision, threads } = expect(await this.conn.request({ type: "threads", session: this.sid }), "threads");
    return { revision, threads };
  }

  /** `revision` is the manifest's; lines of a range target are in its text. */
  async createThread(revision: number, target: ThreadTarget, body: string): Promise<Thread> {
    const request = { type: "createThread", session: this.sid, revision, target, body } as const;
    return asThread(expect(await this.conn.request(request), "thread"));
  }

  async reply(thread: number, body: string): Promise<Thread> {
    const request = { type: "reply", session: this.sid, thread, body } as const;
    return asThread(expect(await this.conn.request(request), "thread"));
  }

  async setThreadStatus(thread: number, status: ThreadStatus): Promise<Thread> {
    const request = { type: "setThreadStatus", session: this.sid, thread, status } as const;
    return asThread(expect(await this.conn.request(request), "thread"));
  }

  async deleteThread(thread: number): Promise<void> {
    expect(await this.conn.request({ type: "deleteThread", session: this.sid, thread }), "done");
  }

  /** Re-anchors an inferred thread where it is now. */
  async confirmAnchor(revision: number, thread: number): Promise<Thread> {
    const request = { type: "confirmAnchor", session: this.sid, revision, thread } as const;
    return asThread(expect(await this.conn.request(request), "thread"));
  }

  /** Re-anchors an ambiguous thread at one of its candidates (an index into its `ambiguous` anchor). */
  async attachAnchor(revision: number, thread: number, candidate: number): Promise<Thread> {
    const request = { type: "attachAnchor", session: this.sid, revision, thread, candidate } as const;
    return asThread(expect(await this.conn.request(request), "thread"));
  }
}
