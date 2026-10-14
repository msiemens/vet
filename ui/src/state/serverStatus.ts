// What the footer says about the language server of the focused file: its name and, as the backend
// has told, whether it is starting, indexing, ready, failed or not installed. Which server serves a
// file, and what each is doing, is the backend's to say; this only words it.

import type { ServerFeatures } from "../generated/ServerFeatures";
import type { ServerInfo } from "../generated/ServerInfo";
import type { ServerRuntime } from "../generated/ServerRuntime";
import type { ServerRuntimes } from "../generated/ServerRuntimes";
import type { Serving } from "../generated/Serving";
import type { Unavailable } from "../generated/Unavailable";
import type { SemanticSummary } from "./semanticSummary";

/**
 * Whether a language server is there for a file served as `serving`: one that was found, whether it
 * runs yet or not. A file not known to be served, or servers not loaded yet, say nothing.
 */
export function hasServer(serving: Serving | undefined, servers: readonly ServerInfo[]): boolean {
  if (serving === undefined) return true;
  if (serving.type === "unsupported") return false;
  const server = servers.find((s) => s.language === serving.language);
  return !server || server.state.type === "found";
}

/**
 * The name of the server of `language` when that server has said it does not offer `feature`; none
 * while it has not said (it has not started yet) or offers it.
 */
export function withoutFeature(
  language: string | undefined,
  servers: readonly ServerInfo[],
  feature: keyof ServerFeatures,
): string | undefined {
  const server = servers.find((s) => s.language === language);
  return server?.features?.[feature] === false ? server.server : undefined;
}

const runtimesOf = (language: string, all: ServerRuntimes) => all.runtimes.filter((r) => r.language === language);

/** Whether a server of `language` is starting or indexing, on either side. */
export function atWork(language: string | undefined, all: ServerRuntimes): boolean {
  return (
    language !== undefined &&
    runtimesOf(language, all).some((r) => r.phase.type === "starting" || r.phase.type === "indexing")
  );
}

/** How far a server is in 0..1; `undefined` when it does not say. */
function fractionOf({ done, total }: { done: number | null; total: number | null }): number | undefined {
  if (done === null || total === null || total <= 0) return undefined;
  return Math.min(1, Math.max(0, done / total));
}

/** The server of `language` as far as it is indexing, on the target side first; none if it does not. */
export function indexingOf(
  language: string | undefined,
  all: ServerRuntimes,
): { fraction?: number; detail?: string } | undefined {
  if (language === undefined) return undefined;
  const own = runtimesOf(language, all).sort(targetFirst);
  for (const { phase } of own) {
    if (phase.type === "indexing") return { fraction: fractionOf(phase), detail: phase.detail ?? undefined };
  }
  return undefined;
}

const targetFirst = (a: ServerRuntime, b: ServerRuntime) => Number(a.side === "base") - Number(b.side === "base");

/** What most deserves saying of a language's servers: work in progress, then a failure, then readiness. */
function mostPressing(own: readonly ServerRuntime[]): ServerRuntime | undefined {
  const order = [...own].sort(targetFirst);
  for (const type of ["indexing", "starting", "failed", "ready"] as const) {
    const found = order.find((r) => r.phase.type === type);
    if (found) return found;
  }
  return undefined;
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

/** A problem with the server that is not the server's own absence or failure is said by its label alone. */
function plain(why: Unavailable): ServerLine {
  return { name: why.label, title: why.message };
}

/**
 * `path` is the focused file tab's file, served as `serving`; the Changes tab holds many files, so
 * no one server's status applies there.
 */
export function serverLine(
  path: string | undefined,
  serving: Serving | undefined,
  servers: readonly ServerInfo[],
  runtimes: ServerRuntimes,
  summary: SemanticSummary,
): ServerLine | undefined {
  if (path === undefined) return undefined;
  const { missing } = summary;
  if (missing && missing.kind !== "serverMissing" && missing.kind !== "serverFailed") return plain(missing);
  const language = missing?.language ?? (serving?.type === "language" ? serving.language : undefined);
  const server = servers.find((s) => s.language === language);
  if (!server) return missing && plain(missing);
  const failed = (why: Unavailable): ServerLine => ({
    name: server.server,
    state: why.kind === "serverMissing" ? "not installed" : "failed",
    title: why.message,
  });
  if (missing) return failed(missing);
  const shown = mostPressing(runtimesOf(server.language, runtimes));
  const side = shown?.side === "base" ? " for the base" : "";
  switch (shown?.phase.type) {
    case "failed":
      return shown.phase.why.kind === "serverMissing" || shown.phase.why.kind === "serverFailed"
        ? failed(shown.phase.why)
        : plain(shown.phase.why);
    case "indexing": {
      const fraction = fractionOf(shown.phase);
      return {
        name: server.server,
        state: fraction === undefined ? "indexing" : `indexing ${Math.round(fraction * 100)}%`,
        title: shown.phase.detail ?? `Indexing ${server.server}${side}`,
        indexing: { fraction },
      };
    }
    case "starting":
      return { name: server.server, state: "starting", title: `Starting ${server.server}${side}` };
    case "ready":
      return { name: server.server, state: "ready" };
    default:
      return { name: server.server };
  }
}
