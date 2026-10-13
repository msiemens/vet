// The DOM side of `gotoLink.ts`: the symbol under the pointer is underlined, and the pointer is a
// hand, while the go to definition modifier is held. The underline is an element in the body over
// the symbol, so it serves text in the editor and the deleted lines drawn outside its document alike.

import { type Extension } from "@codemirror/state";
import { type EditorView, ViewPlugin } from "@codemirror/view";
import { isMac } from "../../../state/host";
import { GotoLink } from "./gotoLink";
import { type HoverAnchor } from "./hoverDom";

/** Mod is Cmd on macOS (Ctrl+click is the context menu there) and Ctrl elsewhere. */
export const isGotoModifier = (e: { metaKey: boolean; ctrlKey: boolean }) => (isMac ? e.metaKey : e.ctrlKey);

/** Set on the editor while one of its symbols is a link. */
const LIVE = "vet-goto-live";

const inside = ({ rect }: HoverAnchor, e: MouseEvent) =>
  e.clientX >= rect.left && e.clientX <= rect.right && e.clientY >= rect.top && e.clientY <= rect.bottom;

/**
 * Underlines the symbol `resolve` finds under the pointer while the modifier is held, if
 * `hasDefinition` says it has a definition.
 */
export function gotoLinks<T extends HoverAnchor>(
  resolve: (view: EditorView, event: MouseEvent) => T | undefined,
  hasDefinition: (anchor: T, signal: AbortSignal) => Promise<boolean>,
): Extension {
  return ViewPlugin.define((view) => {
    /** The last pointer event over the editor, for a modifier pressed while the pointer rests. */
    let pointer: MouseEvent | undefined;
    /** The symbol under the pointer; found only while the modifier is held. */
    let under: T | undefined;
    let modifier = false;
    let line: HTMLElement | undefined;

    const link = new GotoLink<T>({
      hasDefinition,
      show(anchor) {
        line?.remove();
        line = undefined;
        view.dom.classList.toggle(LIVE, anchor !== undefined);
        if (!anchor) return;
        line = document.createElement("div");
        line.className = "vet-goto-line";
        const { left, right, bottom } = anchor.rect;
        line.style.cssText = `left:${left}px;width:${right - left}px;top:${bottom}px`;
        document.body.append(line);
      },
    });

    const refresh = () => {
      if (!modifier) under = undefined;
      else if (pointer && !(under && inside(under, pointer))) under = resolve(view, pointer);
      link.update(under, modifier);
    };
    const reset = () => {
      pointer = undefined;
      under = undefined;
      link.clear();
    };
    const move = (event: MouseEvent) => {
      pointer = event;
      modifier = isGotoModifier(event);
      refresh();
    };
    const key = (event: KeyboardEvent) => {
      modifier = isGotoModifier(event);
      refresh();
    };
    const blur = () => {
      modifier = false;
      refresh();
    };

    view.dom.addEventListener("mousemove", move);
    view.dom.addEventListener("mouseleave", reset);
    view.dom.addEventListener("mousedown", reset, true);
    window.addEventListener("keydown", key, true);
    window.addEventListener("keyup", key, true);
    window.addEventListener("blur", blur);
    document.addEventListener("scroll", reset, true);

    return {
      update(update) {
        if (update.docChanged || update.geometryChanged || update.viewportChanged) reset();
      },
      destroy() {
        reset();
        view.dom.removeEventListener("mousemove", move);
        view.dom.removeEventListener("mouseleave", reset);
        view.dom.removeEventListener("mousedown", reset, true);
        window.removeEventListener("keydown", key, true);
        window.removeEventListener("keyup", key, true);
        window.removeEventListener("blur", blur);
        document.removeEventListener("scroll", reset, true);
      },
    };
  });
}
