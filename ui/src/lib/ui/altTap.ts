// Alt pressed and released on its own is how a Windows app's menu bar is reached; Alt held while
// another key goes down (Alt+1, Alt+←) is a shortcut and must leave the menu alone.

interface AltKeyEvent {
  key: string;
  repeat?: boolean;
  ctrlKey: boolean;
  shiftKey: boolean;
  metaKey: boolean;
}

export class AltTap {
  private pending = false;

  /** Feed every keydown. */
  keydown(e: AltKeyEvent) {
    if (e.key !== "Alt") this.pending = false;
    else if (!e.repeat) this.pending = !(e.ctrlKey || e.shiftKey || e.metaKey);
  }

  /** Feed every keyup; answers whether Alt came up without anything having gone down since it did. */
  keyup(e: Pick<AltKeyEvent, "key">): boolean {
    const tapped = e.key === "Alt" && this.pending;
    this.pending = false;
    return tapped;
  }

  /** The press is no longer a lone Alt: the pointer was used, or the window lost the focus. */
  cancel() {
    this.pending = false;
  }
}
