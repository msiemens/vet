// Typed requests over the connection. The only module besides `connection.ts` that
// knows the wire protocol, so a protocol change lands here.

import { Connection } from "../connection";
import type { Event as ServerEvent } from "../generated/Event";
import type { FileReview } from "../generated/FileReview";
import type { Manifest } from "../generated/Manifest";
import type { NonTextKind } from "../generated/NonTextKind";
import type { Response } from "../generated/Response";

export type FileContents = Extract<Response, { type: "fileContents" }>;
export type NonTextDetail = Extract<Response, { type: "nonTextDetail" }>;

function expect<T extends Response["type"]>(response: Response, type: T): Extract<Response, { type: T }> {
  if (response.type !== type) throw new Error("unexpected response");
  return response as Extract<Response, { type: T }>;
}

export class Backend {
  private constructor(
    private conn: Connection,
    readonly session: number,
  ) {}

  /** Connects and opens a session on the backend's current repository. */
  static async connect(): Promise<Backend> {
    const conn = await Connection.open();
    const opened = expect(await conn.request({ type: "openSession", repo: "." }), "session");
    return new Backend(conn, opened.session);
  }

  onEvent(listener: (event: ServerEvent) => void): () => void {
    return this.conn.onEvent(listener);
  }

  async manifest(): Promise<Manifest> {
    return expect(await this.conn.request({ type: "manifest", session: this.session }), "manifest");
  }

  fileContents(path: string): Promise<FileContents> {
    return this.conn.request({ type: "fileContents", session: this.session, path }).then((r) => expect(r, "fileContents"));
  }

  nonTextDetail(path: string): Promise<NonTextDetail> {
    return this.conn.request({ type: "nonTextDetail", session: this.session, path }).then((r) => expect(r, "nonTextDetail"));
  }

  /** `revision` is the manifest's; the backend rejects a stale one. */
  async setSeen(revision: number, path: string, hunks: number[], seen: boolean): Promise<FileReview> {
    return expect(await this.conn.request({ type: "setSeen", session: this.session, revision, path, hunks, seen }), "review").review;
  }

  async acknowledge(revision: number, path: string, kind: NonTextKind, acknowledged: boolean): Promise<FileReview> {
    return expect(await this.conn.request({ type: "acknowledge", session: this.session, revision, path, kind, acknowledged }), "review").review;
  }

  async hover(path: string, line: number, col: number): Promise<string> {
    const response = await this.conn.request({ type: "hover", session: this.session, path, line, col });
    return response.type === "hover" ? response.text : "";
  }
}
