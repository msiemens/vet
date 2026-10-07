// The header of a file's section: its path, the lines it changes, how much of it is seen and the
// buttons that act on the whole file.

import type { Completion } from "../../../generated/Completion.ts";
import type { FileChange } from "../../../generated/FileChange.ts";
import { basename } from "../../../lib/paths.ts";
import { el, iconButton } from "./dom.ts";

export interface HeaderActions {
  /** Jump to Source at a 0-based target line. */
  openSource(line: number): void;
  comment(): void;
  /** Flips everything in the file. */
  toggleSeen(): void;
  menu(x: number, y: number): void;
}

export interface FileHeader {
  el: HTMLElement;
  /** Where the file's progress is written. */
  count: HTMLElement;
}

/** The progress of a file as the backend counted it: hunks, and non-text items when it has any. */
export function countText(c: Completion): string {
  const parts: string[] = [];
  if (c.hunks || !c.items) parts.push(`${c.seenHunks}/${c.hunks} hunks seen`);
  if (c.items) parts.push(`${c.acknowledgedItems}/${c.items} other`);
  return parts.join(" · ");
}

/** Shows `completion` in the header's count, and marks the section when everything in it is seen. */
export function renderCount(section: { el: HTMLElement; count: HTMLElement }, completion: Completion) {
  section.count.textContent = countText(completion);
  section.el.classList.toggle("all-seen", completion.complete);
}

export function fileHeader(file: FileChange, actions: HeaderActions): FileHeader {
  const header = document.createElement("header");
  const name = basename(file.path);
  const title = el("span", "path mono");
  title.append(el("span", "dir", file.path.slice(0, file.path.length - name.length)), name);
  title.title = file.path;
  const firstLine = file.hunks[0]?.targetStart ?? 0;
  header.addEventListener("dblclick", (e) => {
    if (!(e.target instanceof Element && e.target.closest("button"))) actions.openSource(firstLine);
  });
  const open = iconButton("Open file in tab", "file", () => actions.openSource(firstLine));
  const comment = iconButton(
    "Comment on this file",
    "comments",
    () => actions.comment(),
    "vet-icon-btn vet-review-action",
  );
  const delta = el("span", "delta mono");
  delta.innerHTML = `<span class="add">+${file.delta.added}</span> <span class="del">−${file.delta.removed}</span>`;
  const count = el("span", "count");
  const seen = iconButton(
    "Toggle everything in this file seen",
    "check",
    () => actions.toggleSeen(),
    "vet-icon-btn seen vet-review-action",
    13,
    2,
  );
  const tools = el("span", "tools");
  tools.append(open, comment, seen);
  header.append(title, delta, count, tools);
  header.addEventListener("contextmenu", (e) => {
    e.preventDefault();
    actions.menu(e.clientX, e.clientY);
  });
  return { el: header, count };
}
