// What a hunk bar needs when it is drawn again.

/** What decides a bar's elements and look. */
export interface BarFacts {
  index: number;
  text: string;
  state: string;
  oldBaseLines: number;
  flagged: boolean;
  baseDiffOpen: boolean;
  /** The view's current stop. */
  current: boolean;
  /** The handlers the buttons call, compared by identity. */
  actions: object | null;
}

/**
 * How the drawn bar `from` becomes `to`: not at all, by restyling its outline (it became or ceased to
 * be the current stop), or by drawing it anew. A press on a button makes the hunk current, and the
 * click needs the button it started on, so that change must keep the elements.
 */
export function barChange(from: BarFacts, to: BarFacts): "none" | "outline" | "redraw" {
  if (
    from.index !== to.index ||
    from.text !== to.text ||
    from.state !== to.state ||
    from.oldBaseLines !== to.oldBaseLines ||
    from.flagged !== to.flagged ||
    from.baseDiffOpen !== to.baseDiffOpen ||
    from.actions !== to.actions
  )
    return "redraw";
  return from.current === to.current ? "none" : "outline";
}
