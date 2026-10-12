// How the editor views read review state: they are told which files changed and ask for the state
// themselves. Plain TypeScript, so the views stay free of Svelte; the changeset store implements it.

import type { FileState } from "./reviewModel";

/** Told after review state was written: the paths written, or "all" after a manifest replaced it. Never the state itself. */
export type WriteListener = (paths: ReadonlySet<string> | "all") => void;

export interface ReviewSource {
  /** The review state of a file of the view on screen, as it is now; none for a path the view has no file at. Reads nothing reactively. */
  state(path: string): FileState | undefined;
  /** Calls `listener` after review state was written; returns the unsubscribe function. */
  subscribe(listener: WriteListener): () => void;
}
