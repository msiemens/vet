// A changeset's key as a string: what a command's argument and the screen's memory of each
// changeset are keyed by.

import type { ChangesetKey } from "../generated/ChangesetKey";

export function keyId(key: ChangesetKey): string {
  return key.type === "branch" ? `branch:${key.name}` : "workingTree";
}

/** The key `keyId` made `id` from; `undefined` for anything else. */
export function parseKeyId(id: string | undefined): ChangesetKey | undefined {
  if (id === "workingTree") return { type: "workingTree" };
  if (id?.startsWith("branch:") && id.length > "branch:".length)
    return { type: "branch", name: id.slice("branch:".length) };
  return undefined;
}
