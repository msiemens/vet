// The `.path-text` markup of a path, for code that builds DOM by hand.

import { splitPath } from "./paths.ts";

function build(lead: string, last: string, full: string, className: string): HTMLElement {
  const root = document.createElement("span");
  root.className = `path-text ${className}`.trim();
  root.title = full;
  const dir = document.createElement("span");
  dir.className = "dir";
  dir.textContent = lead;
  const name = document.createElement("span");
  name.className = "name";
  name.textContent = last;
  root.append(dir, name);
  return root;
}

/** A path with its folders dimmed and its file name whole; `suffix` (`:12`) goes with the name. */
export function pathText(path: string, className = "", suffix = ""): HTMLElement {
  const { dir, name } = splitPath(path);
  return build(dir, name + suffix, path, className);
}
