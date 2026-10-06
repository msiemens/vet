// The change card of a non-text item (mode, symlink target, submodule commit, binary, content
// unavailable, rename; SPEC §5.2, §10): a hunk-style header bar with the same "mark seen"
// action, the before and after, and for images a preview strip. Its height is fixed by
// `cardHeight`, so a section can account for it before anything is measured.

import type { BlobContent } from "../../../generated/BlobContent";
import type { EntryDesc } from "../../../generated/EntryDesc";
import type { NonTextItem } from "../../../generated/NonTextItem";
import type { NonTextKind } from "../../../generated/NonTextKind";
import type { Side } from "../../../generated/Side";
import { iconSvg } from "../../../lib/ui/icons";
import { hasPreview } from "./sectionLayout";

const TITLE: Record<NonTextKind, string> = {
  mode: "File mode",
  symlink: "Symlink target",
  submodule: "Submodule commit",
  binary: "Binary file",
  contentUnavailable: "Content unavailable",
  rename: "Renamed",
  empty: "Empty file",
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
    case "empty":
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
  /** Whether the card has a preview strip, which `setPreview` fills. */
  hasPreview: boolean;
  /** Shows the item as acknowledged or not. */
  setAcknowledged(acknowledged: boolean): void;
  /** Marks it as the view's current stop. */
  setCurrent(current: boolean): void;
  /** Fetches and shows the images, or drops them (and their object URLs) again. */
  setPreview(on: boolean): void;
}

export interface CardActions {
  /** Acknowledge or un-acknowledge: the opposite of what the card shows now. */
  toggle(): void;
  comment(): void;
  /** One side of the item's file, for the preview. */
  blob(side: Side): Promise<BlobContent>;
}

const bytesOf = (base64: string): Uint8Array<ArrayBuffer> => Uint8Array.from(atob(base64), (c) => c.charCodeAt(0));

/** Fills a preview thumb from what the backend sent; an SVG is only ever an `<img>` source. */
function showBlob(thumb: HTMLElement, blob: BlobContent, urls: string[]) {
  if (blob.type === "bytes" && blob.mediaType) {
    const url = URL.createObjectURL(new Blob([bytesOf(blob.bytes)], { type: blob.mediaType }));
    urls.push(url);
    const img = document.createElement("img");
    img.alt = "";
    img.src = url;
    thumb.replaceChildren(img);
    thumb.classList.add("image");
  } else if (blob.type === "tooLarge" || blob.type === "unavailable") {
    thumb.replaceChildren(blob.type === "tooLarge" ? "Too large to preview" : "Not available");
    thumb.classList.add("note");
  }
}

export function changeCard(
  path: string,
  oldPath: string | null,
  item: NonTextItem,
  acknowledged: boolean,
  actions: CardActions,
): Card {
  const root = el("article", "vet-card");
  const thumbs: { thumb: HTMLElement; side: Side }[] = [];
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
      for (const [label, side, entry] of [
        ["Before", "base", item.before],
        ["After", "target", item.after],
      ] as const) {
        if (!entry) continue;
        const figure = el("figure", "vet-card-figure");
        const thumb = el("div", "vet-card-thumb");
        figure.append(thumb, el("figcaption", "", label));
        previews.append(figure);
        thumbs.push({ thumb, side });
      }
      root.append(previews);
    }
  }
  const placeholder = (thumb: HTMLElement) => {
    thumb.className = "vet-card-thumb";
    thumb.innerHTML = iconSvg("image", 28, 1.4);
  };
  for (const { thumb } of thumbs) placeholder(thumb);

  let urls: string[] = [];
  let generation = 0;
  const setPreview = (on: boolean) => {
    const mine = ++generation;
    for (const url of urls) URL.revokeObjectURL(url);
    urls = [];
    for (const { thumb, side } of thumbs) {
      placeholder(thumb);
      if (!on) continue;
      // A reply that arrives after the preview was dropped or restarted is ignored.
      actions.blob(side).then(
        (blob) => mine === generation && showBlob(thumb, blob, urls),
        () => undefined,
      );
    }
  };

  const setAcknowledged = (value: boolean) => {
    bar.className = `vet-hunk vet-hunk-${value ? "seen" : "unseen"}`;
    toggle.ariaLabel = toggle.title = value ? "Mark unseen" : "Mark seen";
  };
  setAcknowledged(acknowledged);
  return {
    el: root,
    hasPreview: thumbs.length > 0,
    setPreview,
    setAcknowledged,
    setCurrent: (current) => root.classList.toggle("vet-current", current),
  };
}
