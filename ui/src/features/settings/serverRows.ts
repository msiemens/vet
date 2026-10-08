// What the "Language servers" page shows per language, derived from `ListServers`.

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
  };
}
