// What the footer says about the language server of the focused file: its name and, as far as
// the backend has told, whether it is starting, indexing, ready, failed or not installed.

import type { ServerFeatures } from "../generated/ServerFeatures";
import type { ServerInfo } from "../generated/ServerInfo";
import type { Task } from "../generated/Task";
import { basename } from "../lib/paths.ts";
import type { SemanticSummary } from "./semanticSummary";
import { fractionOf } from "./taskHeadline.ts";

function extensionOf(path: string): string | undefined {
  const name = basename(path);
  const dot = name.lastIndexOf(".");
  return dot > 0 ? name.slice(dot + 1).toLowerCase() : undefined;
}

/**
 * Whether the server serves the file `path`: by its extension, or by its name (`Dockerfile`, also
 * followed by a dot and anything, as `Dockerfile.dev`).
 */
function serves(server: ServerInfo, path: string): boolean {
  const extension = extensionOf(path);
  if (extension !== undefined && server.extensions.includes(extension)) return true;
  const name = basename(path);
  return server.fileNames.some((n) => name === n || name.startsWith(`${n}.`));
}

/**
 * Whether a language server is there for the type of file `path` is: one serving it that was
 * found, whether it runs yet or not. An empty list is one not loaded yet, which says nothing.
 */
export function hasServer(path: string, servers: readonly ServerInfo[]): boolean {
  if (!servers.length) return true;
  return servers.some((s) => s.state.type === "found" && serves(s, path));
}

/** The language whose server serves the type of file `path` is; none if no language does. */
export function languageOf(path: string, servers: readonly ServerInfo[]): string | undefined {
  return servers.find((s) => serves(s, path))?.language;
}

/** Whether the server serving `path` is starting or indexing, by the running background `tasks`. */
export function atWork(path: string, servers: readonly ServerInfo[], tasks: readonly Task[]): boolean {
  const server = servers.find((s) => serves(s, path));
  return (
    !!server &&
    tasks.some((t) => (t.kind === "serverIndex" || t.kind === "serverStart") && t.title.includes(server.server))
  );
}

/**
 * The name of the server for the type of file `path` is when that server has said it does not offer
 * `feature`; none while it has not said (it has not started yet) or offers it.
 */
export function withoutFeature(
  path: string,
  servers: readonly ServerInfo[],
  feature: keyof ServerFeatures,
): string | undefined {
  const server = servers.find((s) => serves(s, path));
  return server?.features?.[feature] === false ? server.server : undefined;
}

export interface ServerLine {
  /** The server's name, or a few words about why there is none. */
  name: string;
  /** What the server is doing, when known: "starting", "indexing 40%", "ready", "failed", "not installed". */
  state?: string;
  /** The backend's explanation, when there is one. */
  title?: string;
  /** Set while the server indexes; `fraction` (0..1) is how far, if it says. */
  indexing?: { fraction?: number };
}

/**
 * `path` is the focused file tab's file; the Changes tab holds many files, so no one server's
 * status applies there. `tasks` are the running background tasks.
 */
export function serverLine(
  path: string | undefined,
  servers: readonly ServerInfo[],
  tasks: readonly Task[],
  summary: SemanticSummary,
): ServerLine | undefined {
  if (path === undefined) return undefined;
  const { missing } = summary;
  if (missing && missing.kind !== "serverMissing" && missing.kind !== "serverFailed") {
    return { name: missing.label, title: missing.message };
  }
  const server = missing?.language
    ? servers.find((s) => s.language === missing.language)
    : servers.find((s) => serves(s, path));
  if (!server) return missing && { name: missing.label, title: missing.message };
  if (missing) {
    return {
      name: server.server,
      state: missing.kind === "serverMissing" ? "not installed" : "failed",
      title: missing.message,
    };
  }
  const own = tasks.filter((t) => t.title.includes(server.server));
  const indexing = own.find((t) => t.kind === "serverIndex");
  if (indexing) {
    const fraction = fractionOf(indexing);
    return {
      name: server.server,
      state: fraction === undefined ? "indexing" : `indexing ${Math.round(fraction * 100)}%`,
      title: indexing.detail ?? indexing.title,
      indexing: { fraction },
    };
  }
  const starting = own.find((t) => t.kind === "serverStart");
  if (starting) return { name: server.server, state: "starting", title: starting.title };
  return { name: server.server, state: summary.answered ? "ready" : undefined };
}
