// A hover popup in the page: the DOM side of `hoverPopup.ts`. The popup is a fixed element in the
// body (a side by side pane clips what it contains) placed above its symbol once, and it stays
// there. One popup is open at a time, whichever editor it belongs to.

import { type Extension } from "@codemirror/state";
import { EditorView, ViewPlugin } from "@codemirror/view";
import { type HoverKeyed, HoverPopup } from "./hoverPopup";

/** A viewport rectangle. */
export interface Rect {
  left: number;
  right: number;
  top: number;
  bottom: number;
}

/** The symbol under the pointer; `rect` is where it is drawn, in viewport coordinates. */
export interface HoverAnchor extends HoverKeyed {
  rect: Rect;
}

const contains = (r: Rect, x: number, y: number) => x >= r.left && x <= r.right && y >= r.top && y <= r.bottom;

/** The popup that is open, of whichever editor. */
let openPopup: { close(): void } | undefined;

/**
 * Hover popups for `view`. `resolve` finds the symbol under a pointer event; `build` makes the
 * popup's element for it, or nothing if there is none to show.
 */
export function hoverPopups<T extends HoverAnchor>(
  resolve: (view: EditorView, event: MouseEvent) => T | undefined,
  build: (anchor: T) => Promise<(() => HTMLElement) | undefined>,
): Extension {
  return ViewPlugin.define((view) => {
    /** The symbol of the last resolved event: while the pointer is inside it, there is nothing to resolve. */
    let last: T | undefined;
    let el: HTMLElement | undefined;

    const onScroll = (event: Event) => {
      if (!(event.target instanceof Node && el?.contains(event.target))) dismiss();
    };
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") dismiss();
    };

    const popup = new HoverPopup<T, () => HTMLElement>({
      query: build,
      open(anchor, create) {
        if (openPopup && openPopup !== popup) openPopup.close();
        const dom = create();
        dom.classList.add("cm-tooltip", "vet-hover-floating");
        dom.addEventListener("mouseenter", onEnter);
        dom.addEventListener("mouseleave", onLeave);
        document.body.append(dom);
        place(dom, anchor.rect);
        el = dom;
        document.addEventListener("scroll", onScroll, true);
        document.addEventListener("keydown", onKey, true);
        const close = () => {
          document.removeEventListener("scroll", onScroll, true);
          document.removeEventListener("keydown", onKey, true);
          dom.remove();
          if (el === dom) el = undefined;
          if (openPopup === popup) openPopup = undefined;
        };
        openPopup = popup;
        return close;
      },
    });
    const dismiss = () => {
      last = undefined;
      popup.dismiss();
    };
    const onEnter = () => popup.pointer(undefined, true);
    const onLeave = (event: MouseEvent) => {
      if (!(event.relatedTarget instanceof Node && view.dom.contains(event.relatedTarget))) popup.pointer(undefined);
    };

    const move = (event: MouseEvent) => {
      if (!(last && contains(last.rect, event.clientX, event.clientY))) last = resolve(view, event);
      popup.pointer(last);
    };
    const leave = (event: MouseEvent) => {
      last = undefined;
      if (event.relatedTarget instanceof Node && el?.contains(event.relatedTarget)) popup.pointer(undefined, true);
      else popup.pointer(undefined);
    };
    const down = () => dismiss();
    view.dom.addEventListener("mousemove", move);
    view.dom.addEventListener("mouseleave", leave);
    view.dom.addEventListener("mousedown", down, true);

    return {
      update(update) {
        if (update.docChanged || update.geometryChanged || update.viewportChanged) last = undefined;
        if (update.docChanged) dismiss();
      },
      destroy() {
        popup.close();
        view.dom.removeEventListener("mousemove", move);
        view.dom.removeEventListener("mouseleave", leave);
        view.dom.removeEventListener("mousedown", down, true);
      },
    };
  });
}

/** Above the symbol, or below it where there is no room above; within the window sideways. */
function place(dom: HTMLElement, rect: Rect) {
  dom.style.left = `${rect.left}px`;
  dom.style.top = `${rect.top - 2}px`;
  const box = dom.getBoundingClientRect();
  if (box.top < 0) {
    dom.style.top = `${rect.bottom + 2}px`;
    dom.style.transform = "none";
  }
  const overflow = box.right - window.innerWidth;
  if (overflow > 0) dom.style.left = `${Math.max(0, rect.left - overflow)}px`;
}
