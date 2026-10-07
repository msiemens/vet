// A changeset's key as a string: what a command's argument and the screen's memory of each
// changeset are keyed by.

import type { ChangesetKey } from "../generated/ChangesetKey";

const BRANCH = "branch:";
const PULL_REQUEST = "pullRequest:";

export function keyId(key: ChangesetKey): string {
  switch (key.type) {
    case "workingTree":
      return "workingTree";
    case "branch":
      return `${BRANCH}${key.name}`;
    case "pullRequest":
      return `${PULL_REQUEST}${key.remote}#${key.number}`;
  }
}

/** The key `keyId` made `id` from; `undefined` for anything else. */
export function parseKeyId(id: string | undefined): ChangesetKey | undefined {
  if (id === "workingTree") return { type: "workingTree" };
  if (id?.startsWith(BRANCH) && id.length > BRANCH.length) return { type: "branch", name: id.slice(BRANCH.length) };
  if (id?.startsWith(PULL_REQUEST)) {
    const rest = id.slice(PULL_REQUEST.length);
    const hash = rest.lastIndexOf("#");
    const number = Number(rest.slice(hash + 1));
    if (hash > 0 && Number.isInteger(number) && number > 0)
      return { type: "pullRequest", remote: rest.slice(0, hash), number };
  }
  return undefined;
}
