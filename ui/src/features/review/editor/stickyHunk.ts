// What stays at the top of a `ChangesetView` while it scrolls: the current file's header and, under
// it, the current hunk's bar.
//
// Only the current section's header is `position: sticky` (the class `pinned`, in `section.css`):
// it stays until the next section pushes it out, and a column of thousands of sticky headers would
// be thousands of nodes for the browser to place on every scroll. The hunk's bar lives inside the
// editor, which draws only what is near the viewport, so a long hunk's bar would be gone long
// before its end. The pin is one element that follows the scroll position, chosen from the hunk
// positions the height model already has.

import type { Mode } from "../../../lib/viewMode.ts";
import { el } from "./dom.ts";
import { HunkBar, hunkHeader } from "./hunkBar.ts";
import { pinnedAt } from "./pinned.ts";
import type { Section } from "./section.ts";

/** What the pin asks of the view that owns it. */
export interface PinHost {
  mode(): Mode;
  /** Whether the hunk is the view's current stop. */
  current(file: number, hunk: number): boolean;
  /** A press on the bar. */
  press(file: number, hunk: number): void;
  /** Another file's header is pinned, or none. */
  pinned(file: number | undefined): void;
}

export class StickyHunk {
  /** The box under the header that clips the bar while it is pushed up. */
  private box = el("div", "vet-pin");
  private bars = el("div", "vet-pin-bars");
  /** The section whose header is pinned. */
  private file?: number;
  private shown?: { file: number; hunk: number };
  private push = 0;

  constructor(
    private sections: readonly Section[],
    private m: { header: number; hunk: number },
    private host: PinHost,
  ) {
    this.box.append(this.bars);
    this.box.addEventListener("pointerdown", (e) => {
      if (!this.shown) return;
      // The press is on the pinned bar, not on the file's other parts.
      e.stopPropagation();
      this.host.press(this.shown.file, this.shown.hunk);
    });
  }

  /** Pins what belongs to the viewport at `y`. */
  update(y: number) {
    const pinned = pinnedAt(this.sections, y, this.m.header, this.m.hunk);
    if (pinned?.file !== this.file) this.host.pinned(pinned?.file);
    this.pinHeader(pinned?.file);
    if (pinned?.hunk === undefined) {
      this.hide();
      return;
    }
    if (this.shown?.file !== pinned.file || this.shown.hunk !== pinned.hunk) this.build(pinned.file, pinned.hunk);
    if (pinned.push !== this.push) {
      this.push = pinned.push;
      this.bars.style.transform = pinned.push ? `translateY(${pinned.push}px)` : "";
    }
  }

  /** The hunk's state or the current stop changed: draws the bar again. */
  refresh() {
    if (this.shown) this.build(this.shown.file, this.shown.hunk);
  }

  destroy() {
    this.pinHeader(undefined);
    this.hide();
  }

  private pinHeader(file: number | undefined) {
    if (file === this.file) return;
    if (this.file !== undefined) this.sections[this.file].el.classList.remove("pinned");
    if (file !== undefined) this.sections[file].el.classList.add("pinned");
    this.file = file;
  }

  private hide() {
    this.box.remove();
    this.shown = undefined;
  }

  private build(file: number, hunk: number) {
    const s = this.sections[file];
    const text = hunkHeader(s.file.hunks[hunk]);
    const current = this.host.current(file, hunk);
    const bar = (actions: Section["hooks"] | null) =>
      new HunkBar(hunk, text, s.review.hunks[hunk], current, actions, this.m.hunk).toDOM();
    // A side by side view has a bar on each side; the base side only shows the header.
    this.bars.replaceChildren(...(this.host.mode() === "split" ? [bar(null), bar(s.hooks)] : [bar(s.hooks)]));
    // The bars in the editor start right of its gutters.
    const gutters = s.handle?.view.dom.style.getPropertyValue("--vet-gutters");
    if (gutters) this.box.style.setProperty("--vet-gutters", gutters);
    this.shown = { file, hunk };
    s.header.append(this.box);
  }
}
