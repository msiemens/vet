// Where the Changes tab was scrolled to when the viewer left, kept per repository and changeset so
// reopening it comes back to the same place.

import { readStored, writeStored } from "../../lib/storage.ts";
import type { ScrollPosition } from "./editor/ChangesetView.ts";

const key = (repo: string, changeset: string) => `vet.changesPosition:${repo}:${changeset}`;

/** A stored position, or none if the text is not one. */
export function parsePosition(text: string | undefined): ScrollPosition | undefined {
  if (text === undefined) return undefined;
  try {
    const value: unknown = JSON.parse(text);
    if (typeof value !== "object" || value === null) return undefined;
    const { path, offset } = value as Record<string, unknown>;
    return typeof path === "string" && typeof offset === "number" && Number.isFinite(offset)
      ? { path, offset }
      : undefined;
  } catch {
    return undefined;
  }
}

export const readPosition = (repo: string, changeset: string) => parsePosition(readStored(key(repo, changeset)));

export const writePosition = (repo: string, changeset: string, position: ScrollPosition) =>
  writeStored(key(repo, changeset), JSON.stringify(position));
