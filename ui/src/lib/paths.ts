// Repository paths are `/`-separated, relative to the root, with `""` for the root itself.

export const joinPath = (dir: string, name: string) => (dir ? `${dir}/${name}` : name);

export const basename = (path: string) => path.slice(path.lastIndexOf("/") + 1);

export const dirname = (path: string) => path.slice(0, Math.max(0, path.lastIndexOf("/")));

/**
 * Follows a chain of directories that each hold nothing but one directory, the way trees show
 * it as one row (`crates/vet-core/src`). `onlyChild` answers the path of a directory's single
 * directory child, or `undefined` if it has anything else. Returns the chain's last path and its names joined.
 */
export function compactChain(
  path: string,
  name: string,
  onlyChild: (path: string) => string | undefined,
): { path: string; name: string } {
  for (let next = onlyChild(path); next !== undefined; next = onlyChild(path)) {
    name += `/${basename(next)}`;
    path = next;
  }
  return { path, name };
}

/** The last folder of a backend-side path, for display; the backend may be Windows. */
export const repoName = (path: string): string => path.split(/[/\\]/).filter(Boolean).at(-1) ?? path;
