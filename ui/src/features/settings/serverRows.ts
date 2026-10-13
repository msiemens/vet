// What the "Language servers" page shows per language, derived from `ListServers`.

import type { ServerFeatures } from "../../generated/ServerFeatures";
import type { ServerInfo } from "../../generated/ServerInfo";
import type { ServerOrigin } from "../../generated/ServerOrigin";
import type { Tsserver } from "../../generated/Tsserver";
import type { InstallProgress } from "../../state/installLog";

/** `installing` and `failed` are the page's own: the protocol only knows found and missing. */
export type ServerStatus = "found" | "custom" | "missing" | "installing" | "failed";

export interface ServerRow {
  language: string;
  title: string;
  server: string;
  status: ServerStatus;
  statusLabel: string;
  path?: string;
  /** Where the found server came from, in full. */
  origin?: string;
  /** Where the found server came from, short enough for the table: the path if it is a custom one. */
  source: string;
  version?: string;
  /** Present for a server that answers through a `tsserver`. */
  tsserver?: Tsserver;
  /** Why the server is missing, or why the install failed. */
  problem?: string;
  /** The path the user set, found or not. */
  customPath?: string;
  /** The version "Install" would install; absent if Vet does not install this server. */
  pinnedVersion?: string;
  canInstall: boolean;
  /** Why "Install" is off, if it is. */
  installBlocker?: string;
  /** What the server offers; `null` until it has started, absent for a missing server. */
  features?: ServerFeature[] | null;
}

export interface ServerFeature {
  name: string;
  offered: boolean;
}

const ORIGINS: Record<ServerOrigin, string> = {
  custom: "Custom path",
  project: "Project toolchain",
  installed: "Installed by Vet",
  path: "PATH",
};

const SHORT_ORIGINS: Record<Exclude<ServerOrigin, "custom">, string> = {
  project: "Project toolchain",
  installed: "Vet",
  path: "PATH",
};

const STATUS_LABELS: Record<ServerStatus, string> = {
  found: "Found",
  custom: "Custom",
  missing: "Missing",
  installing: "Installing…",
  failed: "Install failed",
};

/** The features in the order the page lists them, named as the app names them elsewhere. */
const FEATURES: [keyof ServerFeatures, string][] = [
  ["documentSymbols", "File Structure"],
  ["definition", "Go to Definition"],
  ["references", "Show Usages"],
  ["implementations", "Go to Implementation"],
  ["hover", "Hover"],
  ["callHierarchy", "Call Hierarchy"],
  ["workspaceSymbols", "Symbol search"],
];

/** `undefined` for a missing server (no line); `null` while a found one has not started yet. */
function featureList(info: ServerInfo): ServerFeature[] | null | undefined {
  if (info.state.type === "missing") return undefined;
  const { features } = info;
  if (!features) return null;
  return FEATURES.map(([key, name]) => ({ name, offered: features[key] }));
}

export function serverRow(info: ServerInfo, install?: InstallProgress): ServerRow {
  const { state } = info;
  let status: ServerStatus;
  if (install?.running) status = "installing";
  else if (install?.error !== undefined) status = "failed";
  else if (state.type === "missing") status = "missing";
  else status = state.origin === "custom" ? "custom" : "found";

  const found = state.type === "found" ? state : undefined;
  return {
    language: info.language,
    title: info.title,
    server: info.server,
    status,
    statusLabel: STATUS_LABELS[status],
    path: found?.path,
    origin: found ? ORIGINS[found.origin] : undefined,
    source: !found ? "" : found.origin === "custom" ? found.path : SHORT_ORIGINS[found.origin],
    version: found?.version ?? undefined,
    tsserver: found?.tsserver ?? undefined,
    problem: install?.error ?? (state.type === "missing" ? state.reason : undefined),
    customPath: info.customPath ?? undefined,
    pinnedVersion: info.pinnedVersion ?? undefined,
    canInstall: info.canInstall && !install?.running,
    installBlocker: info.installBlocker ?? undefined,
    features: featureList(info),
  };
}
