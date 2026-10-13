// The change card of a non-text item (mode, symlink target, submodule commit, binary, content
// unavailable, rename; SPEC §5.2, §10): a hunk-style header bar with the same "mark seen"
// action, the before and after, and for images a preview strip. Its height is fixed by
// `cardHeight`, so a section can account for it before anything is measured.

import type { BlobContent } from "../../../generated/BlobContent";
import type { NonTextItem } from "../../../generated/NonTextItem";
import type { Side } from "../../../generated/Side";
import { pathText } from "../../../lib/splitText.ts";
import { iconSvg } from "../../../lib/ui/icons";
import { cardContent } from "./cardContent";
import { el, iconButton } from "./dom";

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
  /** Absent where the view has no comments to offer. */
  comment?(): void;
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
  const toggle = iconButton(
    "Mark seen",
    "check",
    () => actions.toggle(),
    "vet-icon-btn vet-seen-toggle vet-review-action",
    13,
    2,
  );
  const comment =
    actions.comment && iconButton("Comment", "comments", () => actions.comment?.(), "vet-icon-btn vet-review-action");
  const content = cardContent(item, path, oldPath);
  bar.append(el("span", "vet-hunk-text", content.title), pathText(path, "vet-card-path mono"), el("span", "grow"));
  bar.append(...(comment ? [comment] : []), toggle);
  root.append(bar);

  for (const row of content.rows) {
    if (row.type === "change") root.append(line(row.sign, row.text));
    else {
      const note = el("div", "vet-card-line note");
      note.innerHTML = iconSvg("lock", 14, 1.6);
      note.append(row.text);
      root.append(note);
    }
  }
  if (item.hasPreview) {
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
