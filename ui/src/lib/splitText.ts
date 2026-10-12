// The `.path-text` markup of a path or qualified name, for code that builds DOM by hand.

import { splitPath } from "./paths.ts";
import { splitQualified } from "./qualified.ts";

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

/** A qualified name with its qualifier dimmed and its last segment whole. */
export function qualifiedText(name: string, className = ""): HTMLElement {
  const parts = splitQualified(name);
  return build(parts.qualifier, parts.name, name, className);
}
