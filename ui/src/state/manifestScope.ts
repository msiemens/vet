// A manifest is of one view: the changeset, a range of versions, or a commit. Some of what it says is
// the view's own (its base, its files, its completion), some is the changeset's whatever the view.

import type { Base } from "../generated/Base";
import type { Manifest } from "../generated/Manifest";

/** Whether the manifest is of the changeset as a whole, not of a range or a commit. */
export function isChangesetView(manifest: Pick<Manifest, "interdiff">, shownCommit: number | undefined): boolean {
  return !manifest.interdiff && shownCommit === undefined;
}

/** Whether two manifests are of the same revision and the same view, so one holds the other's files at a later time. */
export function sameManifestView(
  held: Pick<Manifest, "revision" | "interdiff"> | undefined,
  heldCommit: number | undefined,
  next: Pick<Manifest, "revision" | "interdiff">,
  nextCommit: number | undefined,
): boolean {
  return (
    held?.revision === next.revision &&
    heldCommit === nextCommit &&
    held.interdiff?.from === next.interdiff?.from &&
    held.interdiff?.to === next.interdiff?.to
  );
}

/**
 * The manifest of the changeset on view as a whole: the one on screen when it is that, else `whole`, if it was
 * read for the revision on screen. None while it is not known.
 */
export function changesetManifest(
  shown: Manifest | undefined,
  shownCommit: number | undefined,
  whole: Manifest | undefined,
): Manifest | undefined {
  if (!shown) return undefined;
  if (isChangesetView(shown, shownCommit)) return shown;
  return whole?.revision === shown.revision ? whole : undefined;
}

/** The changeset's base moved: only the changeset's own manifest says so; a commit's or a range's base is its own. */
export function withChangesetBase(manifest: Manifest, base: Base, shownCommit: number | undefined): Manifest {
  return isChangesetView(manifest, shownCommit) ? { ...manifest, base } : manifest;
}
