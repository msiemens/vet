// What the Find tool says in words, purely.

const plural = (n: number, one: string, many = `${one}s`) => `${n.toLocaleString("en-US")} ${n === 1 ? one : many}`;

const base = (path: string) => path.slice(path.lastIndexOf("/") + 1);

/** The banner of a set whose code has changed since it ran; `files` are those the later versions changed, if known. */
export function staleText(files: readonly string[]): string {
  const names = files.map(base);
  const where = names.length ? ` (${names.join(", ")})` : "";
  return `The code changed since this ran${where}. Rows in changed files may have moved.`;
}

/** What a rerun found, as the banner after it says. */
export const rerunText = ({ fresh, gone }: { fresh: number; gone: number }) => `Rerun: ${fresh} new, ${gone} gone.`;

/** The banner of a list that was cut. */
export const cutText = (shown: number, total: number) =>
  `Showing the first ${shown.toLocaleString("en-US")} of ${total.toLocaleString("en-US")}.`;

/** The status line's count: places, files, and the ones not seen. */
export function countText(places: number, files: number, notSeen: number): string {
  const text = `${plural(places, "result")} in ${plural(files, "file")}`;
  return notSeen ? `${text} · ${notSeen} not seen` : text;
}

/** The tooltip of a tab: what it holds, and whether the next result replaces it. */
export function tabTitle(kind: "usages" | "search", title: string, pinned: boolean): string {
  const what = kind === "usages" ? `Usages of ${title}` : `Find in Files: ${title}`;
  return `${what} · ${pinned ? "pinned" : "replaced by the next result"}`;
}
