// Typed requests over the connection. The only module besides `connection.ts` that
// knows the wire protocol, so a protocol change lands here.

import type { Base } from "../generated/Base";
import type { BaseChoice } from "../generated/BaseChoice";
import type { BranchInfo } from "../generated/BranchInfo";
import type { CommitInfo } from "../generated/CommitInfo";
import type { DirEntry } from "../generated/DirEntry";
import type { Event as ServerEvent } from "../generated/Event";
import type { Manifest } from "../generated/Manifest";
import type { NonTextKind } from "../generated/NonTextKind";
import type { Response } from "../generated/Response";
import type { ReviewUpdate } from "../generated/ReviewUpdate";
import type { TextContent } from "../generated/TextContent";
import { Connection } from "./connection";

export type FileContents = Extract<Response, { type: "fileContents" }>;

export interface BaseCandidates {
  branches: BranchInfo[];
  /** Newest first. */
  commits: CommitInfo[];
}

function expect<T extends Response["type"]>(response: Response, type: T): Extract<Response, { type: T }> {
  if (response.type !== type) throw new Error("unexpected response");
  return response as Extract<Response, { type: T }>;
}

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

  /** The branches and recent commits a base can be picked from. */
  async baseCandidates(): Promise<BaseCandidates> {
    const { branches, commits } = expect(
      await this.conn.request({ type: "listBaseCandidates", session: this.sid }),
      "baseCandidates",
    );
    return { branches, commits };
  }

  /** Hover on the target side at `revision`, the manifest's; empty if the server has nothing to say. */
  async hover(revision: number, path: string, line: number, col: number): Promise<string> {
    const response = await this.conn.request({
      type: "hover",
      session: this.sid,
      revision,
      side: "target",
      path,
      line,
      col,
    });
    return response.type === "hover" ? (response.text ?? "") : "";
  }
}
