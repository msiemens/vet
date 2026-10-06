// What the "Language servers" page shows per language, derived from `ListServers`.

import type { ServerInfo } from "../../generated/ServerInfo";
import type { ServerOrigin } from "../../generated/ServerOrigin";

/** `installing` and `failed` are the page's own: the protocol only knows found and missing. */
export type ServerStatus = "found" | "custom" | "missing" | "installing" | "failed";

/** An install of this language, as this page started it. */
export interface InstallProgress {
  running: boolean;
  /** The backend's message for an install that did not work. */
  error?: string;
}

export interface ServerRow {
  language: string;
  title: string;
  server: string;
  status: ServerStatus;
  statusLabel: string;
  path?: string;
  origin?: string;
  version?: string;
  notes: string[];
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

const TITLES: Record<string, string> = {
  rust: "Rust",
  typescript: "TypeScript / JavaScript",
  python: "Python",
  csharp: "C#",
};

const ORIGINS: Record<ServerOrigin, string> = {
  custom: "Custom path",
  project: "Project toolchain",
  installed: "Installed by Vet",
  path: "On PATH",
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
    title: TITLES[info.language] ?? info.language,
    server: info.server,
    status,
    statusLabel: STATUS_LABELS[status],
    path: found?.path,
    origin: found ? ORIGINS[found.origin] : undefined,
    version: found?.version ?? undefined,
    notes: found?.notes ?? [],
    problem: install?.error ?? (state.type === "missing" ? state.reason : undefined),
    customPath: info.customPath ?? undefined,
    pinnedVersion: info.pinnedVersion ?? undefined,
    canInstall: info.pinnedVersion != null && !info.installBlocker && !install?.running,
    installBlocker: info.installBlocker ?? undefined,
  };
}

/** Keeps the newest `max` lines. */
export function appendLog(lines: readonly string[], line: string, max = 500): string[] {
  const next = [...lines, line];
  return next.length > max ? next.slice(next.length - max) : next;
}
