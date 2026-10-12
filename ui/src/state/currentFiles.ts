import type { FileChange } from "../generated/FileChange";

/**
 * The manifest's files with each one's current state: the manifest's list is as of the last
 * refresh (paths, hunks), the review state of a file is whatever `current` has for it now.
 */
export function currentFiles(files: readonly FileChange[], current: (path: string) => FileChange | undefined) {
  return files.map((f) => current(f.path) ?? f);
}
