// The change card of a non-text item (mode, symlink target, submodule commit, binary, content
// unavailable, rename; SPEC §5.2, §10): a hunk-style header bar with the same "mark seen"
// action, the before and after, and for images a preview strip. Its height is fixed by
// `cardHeight`, so a section can account for it before anything is measured.

import type { EntryDesc } from "../../../generated/EntryDesc";
import type { NonTextItem } from "../../../generated/NonTextItem";
import type { NonTextKind } from "../../../generated/NonTextKind";
import { iconSvg } from "../../../lib/ui/icons";
import { hasPreview } from "./sectionLayout";

const TITLE: Record<NonTextKind, string> = {
  mode: "File mode",
  symlink: "Symlink target",
  submodule: "Submodule commit",
  binary: "Binary file",
  contentUnavailable: "Content unavailable",
  rename: "Renamed",
};

const size = (bytes: number | null): string =>
  bytes === null
    ? ""
    : bytes < 1024
      ? `${bytes} B`
      : bytes < 1048576
        ? `${(bytes / 1024).toFixed(1)} KB`
        : `${(bytes / 1048576).toFixed(1)} MB`;

/** One side of the change as text. */
function describe(kind: NonTextKind, side: EntryDesc | null, path: string): string {
  if (!side) return "(none)";
  switch (kind) {
    case "mode":
      return side.mode.toString(8);
    case "symlink":
      return side.target ?? "";
    case "submodule":
      return side.target?.slice(0, 7) ?? "";
    case "binary":
      return size(side.size);
    case "rename":
      return path;
    default:
      return "";
  }
}

function el<K extends keyof HTMLElementTagNameMap>(tag: K, className: string, text?: string): HTMLElementTagNameMap[K] {
  const e = document.createElement(tag);
  e.className = className;
  if (text !== undefined) e.textContent = text;
  return e;
}

function line(sign: "−" | "+", text: string): HTMLElement {
  const row = el("div", `vet-card-line ${sign === "+" ? "add" : "del"}`);
  row.append(el("span", "sign", sign), el("span", "mono text", text));
  return row;
}

export interface Card {
  el: HTMLElement;
  /** Shows the item as acknowledged or not. */
  setAcknowledged(acknowledged: boolean): void;
  /** Marks it as the view's current stop. */
  setCurrent(current: boolean): void;
}

export interface CardActions {
  /** Acknowledge or un-acknowledge: the opposite of what the card shows now. */
  toggle(): void;
  comment(): void;
}

export function changeCard(
  path: string,
  oldPath: string | null,
  item: NonTextItem,
  acknowledged: boolean,
  actions: CardActions,
): Card {
  const root = el("article", "vet-card");
  const bar = el("div", "vet-hunk");
  const toggle = el("button", "flat vet-hunk-btn vet-seen-toggle");
  toggle.type = "button";
  toggle.innerHTML = iconSvg("check", 13, 2);
  toggle.addEventListener("click", () => actions.toggle());
  const comment = el("button", "flat vet-hunk-btn");
  comment.type = "button";
  comment.ariaLabel = comment.title = "Comment";
  comment.innerHTML = iconSvg("comments", 13, 1.8);
  comment.addEventListener("click", () => actions.comment());
  bar.append(el("span", "vet-hunk-text", TITLE[item.kind]), el("span", "vet-card-path mono", path), el("span", "grow"));
  bar.append(toggle, comment);
  root.append(bar);

  if (item.kind === "contentUnavailable") {
    const note = el("div", "vet-card-line note");
    note.innerHTML = iconSvg("lock", 14, 1.6);
    note.append("The content of this file is not available.");
    root.append(note);
  } else {
    root.append(
      line("−", item.kind === "rename" ? (oldPath ?? "") : describe(item.kind, item.before, path)),
      line("+", describe(item.kind, item.after, path)),
    );
    if (hasPreview(item, path)) {
      const previews = el("div", "vet-card-previews");
      for (const [label, entry] of [
        ["Before", item.before],
        ["After", item.after],
      ] as const) {
        if (!entry) continue;
        const figure = el("figure", "vet-card-figure");
        const thumb = el("div", "vet-card-thumb");
        thumb.innerHTML = iconSvg("image", 28, 1.4);
        figure.append(thumb, el("figcaption", "", label));
        previews.append(figure);
      }
      root.append(previews);
    }
  }

  const setAcknowledged = (value: boolean) => {
    bar.className = `vet-hunk vet-hunk-${value ? "seen" : "unseen"}`;
    toggle.ariaLabel = toggle.title = value ? "Mark unseen" : "Mark seen";
  };
  setAcknowledged(acknowledged);
  return {
    el: root,
    setAcknowledged,
    setCurrent: (current) => root.classList.toggle("vet-current", current),
  };
}
