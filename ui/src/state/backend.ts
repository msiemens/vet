// Typed requests over the connection. The only module besides `connection.ts` that
// knows the wire protocol, so a protocol change lands here.

import { Connection } from "../connection";
import type { FileEntry } from "../generated/FileEntry";
import type { Response } from "../generated/Response";

export type FileContents = Extract<Response, { type: "file" }>;

function expect<T extends Response["type"]>(response: Response, type: T): Extract<Response, { type: T }> {
  if (response.type !== type) throw new Error("unexpected response");
  return response as Extract<Response, { type: T }>;
}

export class Backend {
  private constructor(private conn: Connection) {}

  static async connect(): Promise<Backend> {
    return new Backend(await Connection.open());
  }

  async openChangeset(): Promise<{ name: string; files: FileEntry[] }> {
    return expect(await this.conn.request({ type: "openChangeset" }), "changeset");
  }

  async getFile(path: string): Promise<FileContents> {
    return expect(await this.conn.request({ type: "getFile", path }), "file");
  }

  /** Resolves with the backend's seen state of the whole file. */
  async setSeen(path: string, hunks: number[], seen: boolean): Promise<boolean[]> {
    return expect(await this.conn.request({ type: "setSeen", path, hunks, seen }), "seen").seen;
  }

  async hover(path: string, line: number, col: number): Promise<string> {
    const response = await this.conn.request({ type: "hover", path, line, col });
    return response.type === "hover" ? response.text : "";
  }
}
