// How the footer words the state of syncing with a platform.

import type { PlatformStatus } from "../../generated/PlatformStatus";

const NAMES = { github: "GitHub", azureDevOps: "Azure DevOps" } as const;

export interface ConnectionLabel {
  text: string;
  title: string;
  /** Needs the viewer to sign in again. */
  warn: boolean;
}

export function connectionLabel(status: PlatformStatus): ConnectionLabel {
  const name = NAMES[status.kind];
  const { connection } = status;
  switch (connection.type) {
    case "ok":
      return { text: name, title: `Comments are synced with ${status.host}`, warn: false };
    case "signedOut":
      return { text: `${name} · signed out`, title: `Sign in to ${status.host} to sync comments`, warn: true };
    case "tokenExpired":
      return { text: `${name} · token expired`, title: connection.message, warn: true };
    case "failed":
      return { text: `${name} · not reachable`, title: connection.message, warn: false };
  }
}
