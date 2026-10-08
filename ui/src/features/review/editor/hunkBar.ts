// The hunk header bar: `@@ −40,5 +40,7 @@`, a seen toggle and a comment button.

import { WidgetType } from "@codemirror/view";
import { iconSvg } from "../../../lib/ui/icons";
import { commands } from "../../../state/commands";
import type { Hunk } from "../../../generated/Hunk";
import type { HunkState } from "../../../generated/HunkState";
import { type BarFacts, barChange } from "./barChange";
import { el, iconButton, reconfirmButton } from "./dom";

export interface BarActions {
  /** Flips the hunk; for one seen against an old base that is re-confirming it. */
  toggleSeen(hunk: number): void;
  comment(hunk: number): void;
  /** Jump to Source at the hunk's first line. */
  openSource(hunk: number): void;
  /** Opens or closes the base diff under the bar of a hunk seen against an old base; absent where there is none to show. */
  toggleBaseDiff?(hunk: number): void;
}

/** What a bar shows of its hunk besides the header text. */
export interface BarHunk {
  state: HunkState;
  /** Lines of the hunk covered only by spans seen against an old base. */
  oldBaseLines: number;
  /** Its base diff is open. */
  baseDiffOpen: boolean;
}

export function hunkHeader(h: Hunk): string {
  const start = (s: number, len: number) => (len === 0 ? s : s + 1);
  return `@@ −${start(h.baseStart, h.baseLen)},${h.baseLen} +${start(h.targetStart, h.targetLen)},${h.targetLen} @@`;
}

/** What the seen toggle does to a hunk in each state. */
export const LABEL: Record<HunkState, string> = {
  unseen: "Mark hunk seen",
  seen: "Mark hunk unseen",
  seenAgainstOldBase: "Re-confirm",
  informational: "Nothing left to review",
  unapplied: "Mark hunk seen",
};

/** What an interdiff's hunk says about itself beyond its state. */
const NOTE: Partial<Record<HunkState, string>> = {
  informational: "nothing left to review",
  unapplied: "did not apply to the new base",
};

/** The bar each drawn element shows. */
const DRAWN = new WeakMap<HTMLElement, HunkBar>();

export class HunkBar extends WidgetType {
  /** `actions` is null on the base side of a side-by-side view, which only shows the header. */
  constructor(
    readonly index: number,
    readonly text: string,
    readonly hunk: BarHunk,
    /** The view's current stop: hunk navigation and Space act on it. */
    readonly current: boolean,
    readonly actions: BarActions | null,
    /** The bar's height in px (`--h-hunk`). */
    readonly height: number,
  ) {
    super();
  }

  get state(): HunkState {
    return this.hunk.state;
  }

  /** Exact, so CodeMirror's height map is right before the bar is drawn and scrolling over it reflows nothing. */
  get estimatedHeight(): number {
    return this.height;
  }

  private get facts(): BarFacts {
    const { index, text, state, current, actions } = this;
    const { oldBaseLines, baseDiffOpen } = this.hunk;
    return { index, text, state, oldBaseLines, baseDiffOpen, current, actions };
  }

  eq(other: HunkBar): boolean {
    return barChange(other.facts, this.facts) === "none";
  }

  /** Becoming or ceasing to be the current stop restyles the drawn bar instead of replacing it (`barChange`). */
  updateDOM(dom: HTMLElement): boolean {
    const drawn = DRAWN.get(dom);
    if (!drawn || barChange(drawn.facts, this.facts) === "redraw") return false;
    dom.classList.toggle("vet-hunk-current", this.current);
    DRAWN.set(dom, this);
    return true;
  }

  toDOM(): HTMLElement {
    const bar = document.createElement("div");
    DRAWN.set(bar, this);
    bar.className = `vet-hunk vet-hunk-${this.state}`;
    if (this.current) bar.classList.add("vet-hunk-current");
    const text = document.createElement("span");
    text.className = "vet-hunk-text";
    text.textContent = this.text;
    bar.append(text);
    const actions = this.actions;
    // The base side of a side-by-side view only shows the header.
    if (this.state === "seenAgainstOldBase" && actions) {
      const old = el("button", "flat vet-hunk-old");
      old.type = "button";
      old.title = "Base changed since you marked this seen: show what it was";
      old.innerHTML = `${iconSvg("reconfirm")}<span>${this.hunk.oldBaseLines}</span>`;
      old.ariaPressed = String(this.hunk.baseDiffOpen);
      if (actions.toggleBaseDiff) old.addEventListener("click", () => actions.toggleBaseDiff?.(this.index));
      else old.disabled = true;
      bar.append(old);
    }
    const note = NOTE[this.state];
    if (note) {
      const el = document.createElement("span");
      el.className = "vet-hunk-note";
      el.textContent = note;
      bar.append(el);
    }
    const spacer = document.createElement("span");
    spacer.style.flex = "1";
    bar.append(spacer);
    if (actions) {
      const tools = document.createElement("span");
      tools.className = "vet-hunk-tools";
      tools.append(
        iconButton(commands.tooltip("review.jumpToSource", "Jump to source"), "file", () =>
          actions.openSource(this.index),
        ),
        iconButton("Comment", "comments", () => actions.comment(this.index), "vet-icon-btn vet-review-action"),
        this.state === "seenAgainstOldBase"
          ? reconfirmButton(commands.tooltip("review.reconfirm", "Re-confirm"), () => actions.toggleSeen(this.index))
          : iconButton(
              LABEL[this.state],
              "check",
              () => actions.toggleSeen(this.index),
              "vet-icon-btn vet-seen-toggle vet-review-action",
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
