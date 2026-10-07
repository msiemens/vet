// The base diff under the bar of a hunk seen against an old base: the base-side lines as they were
// when the hunk was marked, as a diff to the lines the base has now. Its height is fixed by
// `baseDiffHeight`, so a section can account for it before anything is measured.

import { WidgetType } from "@codemirror/view";
import type { BaseDiff } from "../../../generated/BaseDiff";
import { baseDiffTitle } from "./baseDiffTitle";
import { el } from "./dom";
import { type Metrics, baseDiffHeight } from "./sectionLayout";

const SIGN = { context: " ", removed: "−", added: "+" } as const;

export class BaseDiffBlock extends WidgetType {
  constructor(
    readonly diff: BaseDiff,
    readonly m: Pick<Metrics, "line" | "diffGap">,
  ) {
    super();
  }

  get estimatedHeight(): number {
    return baseDiffHeight(this.diff.rows.length, this.m);
  }

  eq(other: BaseDiffBlock): boolean {
    return other.diff === this.diff && other.m.line === this.m.line && other.m.diffGap === this.m.diffGap;
  }

  toDOM(): HTMLElement {
    const root = el("div", "vet-basediff");
    root.style.height = `${this.estimatedHeight}px`;
    const box = el("div", "vet-basediff-box");
    box.append(el("div", "vet-basediff-head", baseDiffTitle(this.diff)));
    for (const row of this.diff.rows) {
      const line = el("div", `vet-basediff-row ${row.kind}`);
      line.append(el("span", "no", String(row.line)), el("span", "sign", SIGN[row.kind]), el("span", "text", row.text));
      box.append(line);
    }
    root.append(box);
    return root;
  }

  ignoreEvent(): boolean {
    return true;
  }
}
