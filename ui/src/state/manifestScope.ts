// A manifest is of one view: the changeset, a range of versions, or a commit. Some of what it says is
// the view's own (its base, its files, its completion), some is the changeset's whatever the view.

import type { Manifest } from "../generated/Manifest";

/** Whether the manifest is of the changeset as a whole, not of a range or a commit. */
export function isChangesetView(manifest: Pick<Manifest, "interdiff">, shownCommit: number | undefined): boolean {
  return !manifest.interdiff && shownCommit === undefined;
}
