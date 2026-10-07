// Typed requests over the connection. The only module besides `connection.ts` that
// knows the wire protocol, so a protocol change lands here.

import type { Base } from "../generated/Base";
import type { BaseChoice } from "../generated/BaseChoice";
import type { BlobContent } from "../generated/BlobContent";
import type { BranchInfo } from "../generated/BranchInfo";
import type { ChangedSymbols } from "../generated/ChangedSymbols";
import type { CommentAt } from "../generated/CommentAt";
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
import type { SemanticLocation } from "../generated/SemanticLocation";
import type { ServerInfo } from "../generated/ServerInfo";
import type { Side } from "../generated/Side";
import type { SymbolId } from "../generated/SymbolId";
import type { SymbolSignature } from "../generated/SymbolSignature";
import type { TextContent } from "../generated/TextContent";
import type { Thread } from "../generated/Thread";
import type { ThreadStatus } from "../generated/ThreadStatus";
import type { ThreadTarget } from "../generated/ThreadTarget";
import type { Unavailable } from "../generated/Unavailable";
import { Connection } from "./connection";

export type FileContents = Extract<Response, { type: "fileContents" }>;

/** A language server's Markdown, with the highlights of each of its fenced code blocks, in order. */
export interface HoverText {
  text: string;
  code: number[][];
}

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

  /** The recently opened repositories, most recent first. They belong to the backend, not to a session. */
  async recentProjects(): Promise<string[]> {
    return expect(await this.conn.request({ type: "recentProjects" }), "recentProjects").paths;
  }

  /** Puts `path` at the top of the recent list; resolves with the list. */
  async recordRecentProject(path: string): Promise<string[]> {
    return expect(await this.conn.request({ type: "recordRecentProject", path }), "recentProjects").paths;
  }

  /** Resolves with the recent list without `path`. */
  async forgetRecentProject(path: string): Promise<string[]> {
    return expect(await this.conn.request({ type: "forgetRecentProject", path }), "recentProjects").paths;
  }

  /** Opens a session on `repo`; resolves with the repository root the backend settled on. */
  async open(repo: string): Promise<string> {
    const opened = expect(await this.conn.request({ type: "openSession", repo }), "session");
    this.session = opened.session;
    // The backend resolves a subfolder to its repository root and reports it.
    return opened.root;
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

  /** Marks everything of a file seen, or takes every mark back if it all is seen already. */
  async toggleFileSeen(revision: number, path: string): Promise<ReviewUpdate> {
    return expect(await this.conn.request({ type: "toggleFileSeen", session: this.sid, revision, path }), "review");
  }

  /** Compares against another base; every subscriber, this one included, then gets `snapshotSettled`. */
  async setBase(base: BaseChoice): Promise<Base> {
    return expect(await this.conn.request({ type: "setBase", session: this.sid, base }), "base");
  }

  /** Replaces the open repository's review filter; every subscriber, this one included, then gets `snapshotSettled`. Rejects with the backend's message for an invalid glob. */
  async setProjectFilter(filter: ReviewFilter): Promise<void> {
    expect(await this.conn.request({ type: "setProjectFilter", session: this.sid, filter }), "done");
  }

  /** Excludes a file, or with `directory` everything below a folder, from the review; every subscriber, this one included, then gets `snapshotSettled`. */
  async excludePath(path: string, directory: boolean): Promise<void> {
    expect(await this.conn.request({ type: "excludePath", session: this.sid, path, directory }), "done");
  }

  /** Ends the open review; every subscriber, this one included, then gets `snapshotSettled`. Resolves with whether it was complete. */
  async finishReview(): Promise<boolean> {
    return expect(await this.conn.request({ type: "finishReview", session: this.sid }), "reviewFinished").complete;
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
  ): Promise<HoverText & { unavailable?: Unavailable }> {
    const r = expect(
      await this.conn.request({ type: "hover", session: this.sid, revision, side, path, line, col }),
      "hover",
    );
    return { text: r.text ?? "", code: r.codeHighlights, unavailable: r.unavailable ?? undefined };
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

  /**
   * The diagnostics of a file of the target side, and how many of them to tell the viewer about;
   * the language server may take seconds on a cold start.
   */
  async diagnostics(
    revision: number,
    path: string,
  ): Promise<{ diagnostics: Diagnostic[]; count: number; unavailable?: Unavailable }> {
    const r = expect(
      await this.conn.request({ type: "diagnostics", session: this.sid, revision, path }),
      "diagnostics",
    );
    return { diagnostics: r.diagnostics, count: r.count, unavailable: r.unavailable ?? undefined };
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

  /** The symbols the changeset touches, as of `revision` (the manifest's; a stale one is rejected). */
  async changedSymbols(revision: number): Promise<ChangedSymbols> {
    return expect(await this.conn.request({ type: "changedSymbols", session: this.sid, revision }), "changedSymbols");
  }

  /** The declaration of a changed symbol; `signature` is unset if the server has nothing, `unavailable` says why. */
  async signature(
    revision: number,
    symbol: SymbolId,
  ): Promise<{ signature?: SymbolSignature; unavailable?: Unavailable }> {
    const r = expect(await this.conn.request({ type: "signature", session: this.sid, revision, symbol }), "signature");
    return { signature: r.signature ?? undefined, unavailable: r.unavailable ?? undefined };
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

  /**
   * The line of the target text that a unified diff shows `line` of `side` of the file at `path`
   * (its old path on the base side) at, as of `revision`, the manifest's (a stale one is rejected).
   */
  async targetLine(revision: number, side: Side, path: string, line: number): Promise<number> {
    const request = { type: "targetLine", session: this.sid, revision, side, path, line } as const;
    return expect(await this.conn.request(request), "targetLine").line;
  }

  /**
   * Where a comment on a part of the changeset file at `path` attaches, as of `revision`, the manifest's
   * (a stale one is rejected); `line` is the target line the form of a range target goes below.
   */
  async commentTarget(
    revision: number,
    path: string,
    at: CommentAt,
  ): Promise<{ target: ThreadTarget; line: number | null }> {
    const request = { type: "commentTarget", session: this.sid, revision, path, at } as const;
    const { target, line } = expect(await this.conn.request(request), "commentTarget");
    return { target, line };
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
