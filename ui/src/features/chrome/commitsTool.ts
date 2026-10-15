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

/** One choice seen from two places: the selected tool, the review lens and how many commits there are. */
export interface CommitsChoice {
  tool: string;
  lens: Lens | undefined;
  commits: number;
}

/** The pair to settle on and, when the tool moved, the lens to ask the backend for. */
export interface CommitsSettled extends CommitsChoice {
  askLens?: Lens;
}

/**
 * The Commits tool and the review lens are one choice seen from two places: picking the tool
 * reviews by commit, leaving it returns to all changes, and a lens set elsewhere selects the tool
 * that goes with it. `previous` is the pair the window settled on last, `now` what the stores hold;
 * from that one pair the function decides both directions, so the two never chase each other.
 */
export function settleCommits(previous: CommitsChoice, now: CommitsChoice): CommitsSettled {
  if (now.tool !== previous.tool) {
    // The tool is the source; the lens follows. With nothing to review by commit the tool gives way.
    if (now.tool === COMMITS && now.commits <= 1) return { ...now, tool: FALLBACK };
    return { ...now, askLens: lensOnSelect(previous.tool, now.tool) };
  }
  // The lens, or the commit count, moved: the tool follows the lens.
  return { ...now, tool: toolFor(now.tool, now.lens, now.commits) };
}
