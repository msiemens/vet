// The hunk header bar: `@@ −40,5 +40,7 @@ symbol`, a seen toggle and a comment button.

import { WidgetType } from "@codemirror/view";
import { iconSvg } from "../../../lib/ui/icons";
import type { Hunk } from "../../../generated/Hunk";
import type { HunkState } from "../../../generated/HunkState";

export interface BarActions {
  toggleSeen(hunk: number): void;
  comment(hunk: number): void;
}

export function hunkHeader(h: Hunk, symbol?: string): string {
  const start = (s: number, len: number) => (len === 0 ? s : s + 1);
  const text = `@@ −${start(h.baseStart, h.baseLen)},${h.baseLen} +${start(h.targetStart, h.targetLen)},${h.targetLen} @@`;
  return symbol ? `${text} ${symbol}` : text;
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
    readonly actions: BarActions | null,
  ) {
    super();
  }

  eq(other: HunkBar): boolean {
    return (
      other.index === this.index &&
      other.text === this.text &&
      other.state === this.state &&
      other.actions === this.actions
    );
  }

  toDOM(): HTMLElement {
    const bar = document.createElement("div");
    bar.className = `vet-hunk vet-hunk-${this.state}`;
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
      bar.append(
        this.button(LABEL[this.state], "check", `vet-seen-toggle`, () => actions.toggleSeen(this.index)),
        this.button("Comment", "comments", "", () => actions.comment(this.index)),
      );
    }
    return bar;
  }

  private button(label: string, icon: "check" | "comments", cls: string, onclick: () => void): HTMLElement {
    const b = document.createElement("button");
    b.type = "button";
    b.className = `vet-hunk-btn ${cls}`;
    b.ariaLabel = label;
    b.title = label;
    b.innerHTML = iconSvg(icon, 13, icon === "check" ? 2 : 1.8);
    b.addEventListener("click", onclick);
    return b;
  }

  ignoreEvent(): boolean {
    return true;
  }
}
