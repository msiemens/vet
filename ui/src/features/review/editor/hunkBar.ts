// The hunk header bar: `@@ −40,5 +40,7 @@`, a seen toggle and a comment button.

import { WidgetType } from "@codemirror/view";
import { iconSvg } from "../../../lib/ui/icons";
import { commands } from "../../../state/commands";
import type { Hunk } from "../../../generated/Hunk";
import type { HunkState } from "../../../generated/HunkState";
import { iconButton } from "./dom";

export interface BarActions {
  toggleSeen(hunk: number): void;
  comment(hunk: number): void;
  /** Jump to Source at the hunk's first line. */
  openSource(hunk: number): void;
}

export function hunkHeader(h: Hunk): string {
  const start = (s: number, len: number) => (len === 0 ? s : s + 1);
  return `@@ −${start(h.baseStart, h.baseLen)},${h.baseLen} +${start(h.targetStart, h.targetLen)},${h.targetLen} @@`;
}

const LABEL: Record<HunkState, string> = {
  unseen: "Mark hunk seen",
  seen: "Mark hunk unseen",
  seenAgainstOldBase: "Re-confirm seen",
};

export class HunkBar extends WidgetType {
  /** `actions` is null on the base side of a side-by-side view, which only shows the header. */
  constructor(
    readonly index: number,
    readonly text: string,
    readonly state: HunkState,
    /** The view's current stop: hunk navigation and Space act on it. */
    readonly current: boolean,
    readonly actions: BarActions | null,
    /** The bar's height in px (`--h-hunk`). */
    readonly height: number,
  ) {
    super();
  }

  /** Exact, so CodeMirror's height map is right before the bar is drawn and scrolling over it reflows nothing. */
  get estimatedHeight(): number {
    return this.height;
  }

  eq(other: HunkBar): boolean {
    return (
      other.index === this.index &&
      other.text === this.text &&
      other.state === this.state &&
      other.current === this.current &&
      other.actions === this.actions
    );
  }

  toDOM(): HTMLElement {
    const bar = document.createElement("div");
    bar.className = `vet-hunk vet-hunk-${this.state}`;
    if (this.current) bar.classList.add("vet-hunk-current");
    const text = document.createElement("span");
    text.className = "vet-hunk-text";
    text.textContent = this.text;
    bar.append(text);
    if (this.state === "seenAgainstOldBase") {
      const old = document.createElement("span");
      old.className = "vet-hunk-old";
      old.title = "Base changed since you marked this seen";
      old.innerHTML = iconSvg("reconfirm");
      bar.append(old);
    }
    const spacer = document.createElement("span");
    spacer.style.flex = "1";
    bar.append(spacer);
    const actions = this.actions;
    if (actions) {
      const tools = document.createElement("span");
      tools.className = "vet-hunk-tools";
      tools.append(
        iconButton(commands.tooltip("review.jumpToSource", "Jump to source"), "file", () =>
          actions.openSource(this.index),
        ),
        iconButton("Comment", "comments", () => actions.comment(this.index)),
        iconButton(
          LABEL[this.state],
          "check",
          () => actions.toggleSeen(this.index),
          "vet-icon-btn vet-seen-toggle",
          13,
          2,
        ),
      );
      bar.append(tools);
    }
    return bar;
  }

  ignoreEvent(): boolean {
    return true;
  }
}
