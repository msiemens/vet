// The Commits tool and the lens go together: picking the tool reviews the changeset by commit,
// leaving it returns to all changes.

import type { Lens } from "../../generated/Lens";

/** The id of the Commits tool. */
export const COMMITS = "commits";

/** The tool the review falls back to when the Commits tool has nothing to show. */
const FALLBACK = "changes";

/** The lens that selecting `to` instead of `from` asks for; none when it leaves the lens as it is. */
export function lensOnSelect(from: string, to: string): Lens | undefined {
  if (to === COMMITS && from !== COMMITS) return "perCommit";
  if (from === COMMITS && to !== COMMITS) return "squash";
  return undefined;
}

/**
 * The tool selected once the changeset is reviewed in `lens` and has `commits` commits: a lens
 * that was set elsewhere (the changeset reopened by commit) selects the Commits tool, and the tool
 * gives way when there is nothing to review by commit.
 */
export function toolFor(tool: string, lens: Lens | undefined, commits: number): string {
  if (tool === COMMITS && (lens === "squash" || commits <= 1)) return FALLBACK;
  if (tool !== COMMITS && lens === "perCommit" && commits > 1) return COMMITS;
  return tool;
}
