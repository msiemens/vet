import { ViewPlugin, type ViewUpdate } from "@codemirror/view";

/**
 * Publishes the gutters' width as `--vet-gutters`, for what reaches back over them
 * (thread cards, the strip of a deleted block). It is read in the measure phase,
 * where CodeMirror's own reads are batched, and only when the gutters can have
 * changed: a height-only update must not ask for a measure, which restarts the loop.
 */
export const gutterWidth = ViewPlugin.define((view) => {
  const measure = () =>
    view.requestMeasure({
      read: () => view.dom.querySelector<HTMLElement>(".cm-gutters")?.offsetWidth ?? 0,
      write: (width) => view.dom.style.setProperty("--vet-gutters", `${width}px`),
      key: gutterWidth,
    });
  measure();
  return { update: (u: ViewUpdate) => (u.docChanged || u.viewportChanged) && measure() };
});
