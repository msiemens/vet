// Repository paths are `/`-separated, relative to the root, with `""` for the root itself.

export const joinPath = (dir: string, name: string) => (dir ? `${dir}/${name}` : name);

export const basename = (path: string) => path.slice(path.lastIndexOf("/") + 1);

export const dirname = (path: string) => path.slice(0, Math.max(0, path.lastIndexOf("/")));

/** The `paths` inside the directory `dir`, at any depth; a directory is not its own sibling's prefix (`src` does not hold `src2/a`). */
export const pathsBelow = (paths: readonly string[], dir: string): string[] =>
  paths.filter((path) => path.startsWith(`${dir}/`));

/**
 * A path's folders with their trailing separator and its last segment, the parts `.path-text`
 * shortens apart. Splits on `/` and on `\` (a backend-side path may be Windows') and keeps a
 * trailing separator with the last segment, so a folder shows as `/home/me/` + `logs/`.
 */
export const splitPath = (path: string): { dir: string; name: string } => {
  const end = path.search(/[/\\]*$/);
  if (end === 0) return { dir: "", name: path };
  const cut = Math.max(path.lastIndexOf("/", end - 1), path.lastIndexOf("\\", end - 1)) + 1;
  return { dir: path.slice(0, cut), name: path.slice(cut) };
};

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
