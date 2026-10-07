// The popover a change mark opens in a file's text view: the hunk's header, the lines it removed
// or replaced, and the hunk bar's actions. One popover at a time; any press outside it, Escape or
// a scroll closes it, and so does choosing an action.

import type { Hunk } from "../../../generated/Hunk";
import type { HunkState } from "../../../generated/HunkState";
import { el } from "./dom";
import { LABEL, hunkHeader } from "./hunkBar";

/** What the owner does for a hunk of the file, by the hunk's index. */
export interface HunkActions {
  toggleSeen(hunk: number): void;
  comment(hunk: number): void;
  /** Opens the file's diff at the hunk. */
  openInDiff(hunk: number): void;
}

export interface PopoverHunk {
  index: number;
  hunk: Hunk;
  state: HunkState;
  /** The lines the hunk removed or replaced; loaded when the popover opens. */
  oldLines: () => Promise<string[]>;
}

const NOTE: Partial<Record<HunkState, string>> = {
  seen: "seen",
  seenAgainstOldBase: "seen against an older base",
  informational: "nothing left to review",
  unapplied: "did not apply to the new base",
};

let close: (() => void) | undefined;

export function closeHunkPopover() {
  close?.();
}

/** Shows the popover beside `anchor`, replacing the one that is open. */
export function openHunkPopover(anchor: HTMLElement, h: PopoverHunk, actions: HunkActions) {
  close?.();
  const pop = el("div", "surface vet-hunk-pop");
  pop.setAttribute("role", "dialog");
  const head = el("div", "vet-hunk-pop-head");
  head.append(el("span", `vet-hunk-pop-state vet-mark-${h.state}`), el("span", "vet-hunk-text", hunkHeader(h.hunk)));
  const note = NOTE[h.state];
  if (note) head.append(el("span", "vet-hunk-note", note));
  pop.append(head);

  const old = el("pre", "vet-old-lines");
  if (h.hunk.baseLen === 0) {
    old.textContent = "Added: there are no old lines.";
    old.classList.add("vet-old-none");
  } else {
    old.textContent = "Loading the old lines…";
    old.classList.add("vet-old-none");
    void h.oldLines().then(
      (lines) => {
        old.classList.remove("vet-old-none");
        old.textContent = lines.join("\n");
      },
      () => {
        old.textContent = "The old lines could not be loaded.";
      },
    );
  }
  pop.append(old);

  const run = (action: () => void) => () => {
    close?.();
    action();
  };
  const tools = el("div", "vet-hunk-pop-tools");
  const button = (label: string, action: () => void, className = "") => {
    const b = el("button", `ctl vet-review-action ${className}`.trim(), label);
    b.type = "button";
    b.addEventListener("click", run(action));
    return b;
  };
  if (h.state !== "informational") tools.append(button(LABEL[h.state], () => actions.toggleSeen(h.index)));
  tools.append(
    button("Comment", () => actions.comment(h.index)),
    button("Open in diff", () => actions.openInDiff(h.index)),
  );
  pop.append(tools);

  document.body.append(pop);
  const rect = anchor.getBoundingClientRect();
  const left = Math.min(rect.right + 6, window.innerWidth - pop.offsetWidth - 4);
  const top = Math.min(rect.top, window.innerHeight - pop.offsetHeight - 4);
  pop.style.left = `${Math.max(4, left)}px`;
  pop.style.top = `${Math.max(4, top)}px`;

  const onPress = (e: Event) => e.target instanceof Node && !pop.contains(e.target) && close?.();
  const onKey = (e: KeyboardEvent) => e.key === "Escape" && close?.();
  const onScroll = (e: Event) => e.target instanceof Node && !pop.contains(e.target) && close?.();
  window.addEventListener("pointerdown", onPress, true);
  window.addEventListener("keydown", onKey, true);
  window.addEventListener("scroll", onScroll, true);
  window.addEventListener("resize", dismiss);
  function dismiss() {
    window.removeEventListener("pointerdown", onPress, true);
    window.removeEventListener("keydown", onKey, true);
    window.removeEventListener("scroll", onScroll, true);
    window.removeEventListener("resize", dismiss);
    pop.remove();
    if (close === dismiss) close = undefined;
  }
  close = dismiss;
}
