// What the backend's manifest says, in the three scopes it mixes. The generated `Manifest` and
// `FileChange` carry the view's structure, the live review state and the changeset's facts in one
// shape; only `changeset.svelte.ts` and the backend client handle them. Everything else reads these
// types, so a snapshot cannot be taken for the live review state.

import type { AnalysisTrust } from "../generated/AnalysisTrust";
import type { Base } from "../generated/Base";
import type { Completion } from "../generated/Completion";
import type { ExcludedGroup } from "../generated/ExcludedGroup";
import type { FileChange } from "../generated/FileChange";
import type { FileReview } from "../generated/FileReview";
import type { InterdiffInfo } from "../generated/InterdiffInfo";
import type { Lens } from "../generated/Lens";
import type { LineDelta } from "../generated/LineDelta";
import type { Manifest } from "../generated/Manifest";
import type { ReviewFilter } from "../generated/ReviewFilter";
import type { ReviewInfo } from "../generated/ReviewInfo";
import type { Target } from "../generated/Target";
import type { WatchDegraded } from "../generated/WatchDegraded";
import type { Shown } from "./shown";
import { CHANGESET, commitView, interdiffView } from "./view";

/** A file's structure in one view: what only a new manifest changes. It has no review state. */
export type FileLayout = Readonly<Omit<FileChange, "review" | "completion">>;

/** A file's live review state. */
export interface FileState {
  readonly review: FileReview;
  readonly completion: Completion;
}

/** A file with its review state as it is at the moment of reading; never kept. */
export type ReviewedFile = FileLayout & FileState;

/** What the manifest says about the view that only a refresh changes. */
export interface ViewLayout {
  readonly shown: Shown;
  readonly files: readonly FileLayout[];
  readonly paths: readonly string[];
  readonly delta: LineDelta;
  /** The view's base: a commit's parent, an interdiff's base, or the changeset's. */
  readonly base: Base;
  readonly interdiff?: InterdiffInfo;
  readonly excludedLines: number;
  readonly excludedItems: number;
  /** The files the review leaves out, by the reason. */
  readonly excluded: readonly ExcludedGroup[];
  readonly degraded?: WatchDegraded;
}

/** What is true of the changeset as a whole, whatever view is shown. */
export interface ChangesetFacts {
  readonly target: Target;
  /** The changeset's own base, which the header names where it is not the default branch's. */
  readonly base: Base;
  readonly review: ReviewInfo;
  /** The project's filter. */
  readonly filter: ReviewFilter;
  /** The filter this review is narrowed to, which replaces the project's until reset. */
  readonly narrowing?: ReviewFilter;
  /** What analysis of the changeset may execute: the project's trust, and a pull request's own. */
  readonly trust: AnalysisTrust;
  readonly lens: Lens;
  readonly commits: number;
  /** How many files the changeset changes. */
  readonly files: number;
  readonly completion: Completion;
}

/** What a refresh of the view says that only a refresh changes. */
export interface Frame {
  readonly layout: ViewLayout;
  readonly facts: ChangesetFacts;
}

/** A manifest taken apart. */
export interface Parts extends Frame {
  /** The completion of the view. */
  readonly completion: Completion;
  readonly states: ReadonlyMap<string, FileState>;
  /** The project's change counter as of the state the manifest was read from. */
  readonly seq: number;
  readonly revision: number;
}

/** Which view a manifest is of. */
export function viewOf(manifest: Pick<Manifest, "interdiff">, commit: number | undefined): Shown["view"] {
  if (manifest.interdiff) return interdiffView(manifest.interdiff.from, manifest.interdiff.to);
  return commit === undefined ? CHANGESET : commitView(commit);
}

const layoutOf = (file: FileChange): FileLayout => ({
  path: file.path,
  oldPath: file.oldPath,
  status: file.status,
  baseLines: file.baseLines,
  targetLines: file.targetLines,
  hunks: file.hunks,
  delta: file.delta,
  nonText: file.nonText,
});

/** The manifest of the view at `commit` (none: not by commit), taken apart by copying fields out, not by casting. */
export function partsOf(manifest: Manifest, commit: number | undefined): Parts {
  const files = manifest.files.map(layoutOf);
  return {
    layout: {
      shown: { revision: manifest.revision, view: viewOf(manifest, commit) },
      files,
      paths: files.map((f) => f.path),
      delta: manifest.delta,
      base: manifest.base,
      interdiff: manifest.interdiff,
      excludedLines: manifest.excludedLines,
      excludedItems: manifest.excludedItems,
      excluded: manifest.excluded,
      degraded: manifest.degraded,
    },
    facts: {
      target: manifest.target,
      base: manifest.changesetBase,
      review: manifest.review,
      filter: manifest.filter,
      narrowing: manifest.narrowing,
      trust: manifest.trust,
      lens: manifest.lens,
      commits: manifest.commits,
      files: manifest.changesetFiles,
      completion: manifest.changesetCompletion,
    },
    completion: manifest.completion,
    states: new Map(manifest.files.map((f) => [f.path, { review: f.review, completion: f.completion }])),
    seq: manifest.seq,
    revision: manifest.revision,
  };
}
