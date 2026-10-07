// Which commit the review view shows while the changeset is reviewed by commit.

import type { Lens } from "../generated/Lens";

/**
 * The commit to show for a changeset the backend says is reviewed in `lens`, with `commits` commits
 * in all and `last` the one that was on view: none in the squash lens, and none for a changeset of
 * one commit or none, which has no commits to go through.
 */
export function commitToShow(lens: Lens, commits: number, last: number): number | undefined {
  return lens === "perCommit" && commits > 1 ? Math.min(Math.max(last, 0), commits - 1) : undefined;
}

/**
 * The commit `delta` away from `from`, or `undefined` at the end of the list. A step off an end is
 * not wrapped: the last commit's last hunk is where the review ends.
 */
export function neighbour(from: number, delta: number, commits: number): number | undefined {
  const to = from + delta;
  return to >= 0 && to < commits ? to : undefined;
}
