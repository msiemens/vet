// The Commits tool and the lens go together: picking the tool reviews the changeset by commit,
// leaving it returns to all changes. The lens the backend keeps is the source; the rail reads it.

import type { Lens } from "../../generated/Lens";

/** The id of the Commits tool. */
export const COMMITS = "commits";

/** The tool the Commits tool gives way to when there is nothing to review by commit. */
export const FALLBACK = "changes";

/** The lens that selecting `to` from `from` asks for; none when it leaves the lens as it is. */
export function lensOnSelect(from: string, to: string): Lens | undefined {
  if (to === COMMITS && from !== COMMITS) return "perCommit";
  if (from === COMMITS && to !== COMMITS) return "squash";
  return undefined;
}

/**
 * The left tool to show for `tool`, the tool chosen last, while the changeset is reviewed in `lens`
 * and has `commits` commits: the Commits tool while the review is by commit, the chosen tool
 * otherwise. The lens is what the backend keeps, so a lens set elsewhere selects the tool with it.
 */
export function toolFor(tool: string, lens: Lens | undefined, commits: number): string {
  return lens === "perCommit" && commits > 1 ? COMMITS : tool;
}
