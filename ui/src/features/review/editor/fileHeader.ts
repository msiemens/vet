// The header of a file's section: its path, the lines it changes, how much of it is seen and the
// buttons that act on the whole file.

import type { Completion } from "../../../generated/Completion.ts";
import type { FileChange } from "../../../generated/FileChange.ts";
import type { FileReview } from "../../../generated/FileReview.ts";
import { splitPath } from "../../../lib/paths.ts";
import { el, iconButton, reconfirmButton } from "./dom.ts";

export interface HeaderActions {
  /** Jump to Source at a 0-based target line. */
  openSource(line: number): void;
  comment(): void;
  /** Flips everything in the file. */
  toggleSeen(): void;
  /** Marks the hunks seen against an old base seen again. */
  reconfirm(): void;
  menu(x: number, y: number): void;
}

/** The parts of a file header that show its progress. */
export interface FileProgress {
  /** Where the file's progress is written. */
  count: HTMLElement;
  /** The meter's blue part (seen) and the striped part after it (seen against an old base). */
  meter: { seen: HTMLElement; old: HTMLElement };
  /** Offered while the file has hunks seen against an old base. */
  reconfirm: HTMLElement;
}

export interface FileHeader extends FileProgress {
  el: HTMLElement;
}

/** The progress of a file as the backend counted it: hunks, and non-text items when it has any. */
export function countText(c: Completion): string {
  const parts: string[] = [];
  if (c.hunks || !c.items) parts.push(`${c.seenHunks}/${c.hunks} hunks seen`);
  if (c.items) parts.push(`${c.acknowledgedItems}/${c.items} other`);
  return parts.join(" · ");
}

/** Shows a file's progress in its header, and marks the section when everything in it is seen. */
export function renderProgress(
  section: FileProgress & { el: HTMLElement },
  review: FileReview,
  completion: Completion,
) {
  section.count.textContent = countText(completion);
  section.el.classList.toggle("all-seen", completion.complete);
  section.meter.seen.style.width = `${completion.seenPercent}%`;
  section.meter.old.style.width = `${completion.oldBasePercent}%`;
  section.reconfirm.hidden = !review.hunks.includes("seenAgainstOldBase");
}

export function fileHeader(file: FileChange, actions: HeaderActions): FileHeader {
  const header = document.createElement("header");
  const { dir, name } = splitPath(file.path);
  const title = el("span", "path path-text mono");
  title.append(el("span", "dir", dir), el("span", "name", name));
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
  const meter = { seen: el("span", "seen"), old: el("span", "old") };
  const bar = el("span", "meter");
  bar.append(meter.seen, meter.old);
  const reconfirm = reconfirmButton("Re-confirm every hunk of this file seen against an old base", () =>
    actions.reconfirm(),
  );
  const tools = el("span", "tools");
  tools.append(open, comment, reconfirm, seen);
  header.append(title, delta, count, bar, tools);
  header.addEventListener("contextmenu", (e) => {
    e.preventDefault();
    actions.menu(e.clientX, e.clientY);
  });
  return { el: header, count, meter, reconfirm };
}
