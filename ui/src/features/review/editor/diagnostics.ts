// The target-side diagnostics an editor draws: underlines and a gutter dot, but nothing inside a
// collapsed region. The drawn decorations are a state field derived from the stored answers and the
// collapse field, so a collapse that changes the visible text changes the marks in the same
// transaction instead of dispatching an update from a measure.

import { type Diagnostic as LintDiagnostic } from "@codemirror/lint";
import { RangeSetBuilder, StateEffect, StateField, type Text } from "@codemirror/state";
import { Decoration, type DecorationSet, EditorView, GutterMarker, WidgetType, gutter } from "@codemirror/view";
import type { Diagnostic } from "../../../generated/Diagnostic";
import type { Severity } from "../../../generated/Severity";
import { severitySvg } from "../../../lib/ui/severity.ts";
import { collapsedField, hiddenRegions } from "./collapse.ts";
import { type Span, outsideHidden } from "./hiddenRanges.ts";
import { offsetAt } from "./positions.ts";

/** The class of the mark of a diagnostic that was already there on the base. */
const EXISTING = "vet-existing";

const SEVERITY_RANK: Record<Severity, number> = { error: 0, warning: 1, info: 2, hint: 3 };

/** Replaces the whole list of the server's answers; the drawn part is derived from it. */
export const setDiagnosticList = StateEffect.define<readonly Diagnostic[]>();
/** The diagnostics the server gave, whole: the ones drawn are those outside the collapsed regions of the moment. */
export const diagnosticList = StateField.define<readonly Diagnostic[]>({
  create: () => [],
  update(list, tr) {
    for (const e of tr.effects) if (e.is(setDiagnosticList)) return e.value;
    return list;
  },
});

/**
 * The drawn diagnostics, derived from the stored list and the collapsed regions: a collapse or an
 * answer changes the state, and the decorations follow it without dispatching an update of their own.
 */
export const diagnosticMarks = StateField.define<DecorationSet>({
  create: (state) => drawDiagnostics(state.field(diagnosticList, false) ?? [], hiddenRegions(state), state.doc),
  update(marks, tr) {
    const list = tr.state.field(diagnosticList, false);
    const collapsed = tr.state.field(collapsedField, false);
    if (list === tr.startState.field(diagnosticList, false) && collapsed === tr.startState.field(collapsedField, false))
      return marks;
    return drawDiagnostics(list ?? [], hiddenRegions(tr.state), tr.state.doc);
  },
  provide: (f) => EditorView.decorations.from(f),
});

/** Replaces the editor's diagnostics; the drawn part follows the collapsed regions from the state. */
export function showDiagnostics(view: EditorView, diagnostics: readonly Diagnostic[]) {
  view.dispatch({ effects: setDiagnosticList.of(diagnostics) });
}

/** The marks and points of the stored diagnostics outside `hidden`, merged so overlapping problems share one mark. */
export function drawDiagnostics(stored: readonly Diagnostic[], hidden: readonly Span[], doc: Text): DecorationSet {
  return lintDecorations(
    stored.flatMap((d): LintDiagnostic[] => {
      const from = offsetAt(doc, d.span.start);
      const span = { from, to: Math.max(from, offsetAt(doc, d.span.end)) };
      return outsideHidden(span, hidden).map((piece) => ({
        ...piece,
        severity: d.severity,
        message: d.message,
        source: [d.source, d.code].filter(Boolean).join(" ") || undefined,
        markClass: d.origin === "existing" ? EXISTING : undefined,
      }));
    }),
    doc,
  );
}

/** The gutter's mark for a line's worst diagnostic, in the shape of its severity: strong for one the change introduced, faded for one already there. */
class DiagnosticDot extends GutterMarker {
  private readonly severity: Severity;
  private readonly existing: boolean;

  constructor(severity: Severity, existing: boolean) {
    super();
    this.severity = severity;
    this.existing = existing;
  }

  eq(other: GutterMarker) {
    return other instanceof DiagnosticDot && other.severity === this.severity && other.existing === this.existing;
  }

  toDOM() {
    const mark = document.createElement("span");
    mark.className = `vet-diag-dot${this.existing ? " existing" : ""}`;
    mark.innerHTML = severitySvg(this.severity);
    return mark;
  }
}

/** The point drawn for a diagnostic of no width or one too short to underline. */
class DiagnosticPoint extends WidgetType {
  private readonly severity: Severity;

  constructor(severity: Severity) {
    super();
    this.severity = severity;
  }

  eq(other: WidgetType) {
    return other instanceof DiagnosticPoint && other.severity === this.severity;
  }

  toDOM() {
    const point = document.createElement("span");
    point.className = `cm-lintPoint cm-lintPoint-${this.severity}`;
    return point;
  }
}

interface Found {
  severity: Severity;
  existing: boolean;
}

export const diagnosticGutter = gutter({
  class: "vet-diag-gutter",
  lineMarker(view, line) {
    let worst: Found | undefined;
    view.state.field(diagnosticMarks).between(0, 1e9, (_, __, deco) => {
      for (const d of (deco.spec as { diagnostics?: readonly LintDiagnostic[] }).diagnostics ?? []) {
        // A dot for the line a diagnostic starts on, as `forEachDiagnostic` of `@codemirror/lint` reports it.
        if (d.from < line.from || d.from > line.to) continue;
        const found: Found = { severity: d.severity, existing: d.markClass === EXISTING };
        const rank = (x: Found) => SEVERITY_RANK[x.severity] * 2 + (x.existing ? 1 : 0);
        if (!worst || rank(found) < rank(worst)) worst = found;
      }
    });
    const dot: Found | undefined = worst;
    return dot ? new DiagnosticDot(dot.severity, dot.existing) : null;
  },
  lineMarkerChange: (update) => update.startState.field(diagnosticMarks) !== update.state.field(diagnosticMarks),
});

/** Merges the drawn diagnostics into the non-overlapping marks and points `@codemirror/lint` would draw. */
export function lintDecorations(list: readonly LintDiagnostic[], doc: Text): DecorationSet {
  const sorted = list.slice().sort((a, b) => a.from - b.from || a.to - b.to);
  const builder = new RangeSetBuilder<Decoration>();
  let active: LintDiagnostic[] = [];
  let pos = 0;
  const scan = doc.iter();
  let scanPos = 0;
  const docLen = doc.length;
  for (let i = 0; ;) {
    const next = i === sorted.length ? null : sorted[i];
    if (!next && !active.length) break;
    let from: number;
    let to: number;
    if (active.length) {
      from = pos;
      to = active.reduce((p, d) => Math.min(p, d.to), next && next.from > from ? next.from : 1e8);
    } else if (next) {
      from = next.from;
      // A diagnostic past the end of the document has nothing to mark.
      if (from > docLen) break;
      to = next.to;
      active.push(next);
      i++;
    } else {
      break;
    }
    while (i < sorted.length) {
      const following = sorted[i];
      if (following.from === from && (following.to > following.from || following.to === from)) {
        active.push(following);
        i++;
        to = Math.min(following.to, to);
      } else {
        to = Math.min(following.from, to);
        break;
      }
    }
    to = Math.min(to, docLen);
    let point = false;
    if (active.some((d) => d.from === from && (d.to === to || to === docLen))) {
      point = from === to;
      if (!point && to - from < 10) {
        // Too short to underline unambiguously: a point, unless it spans a line break or a tab.
        const behind = from - (scanPos + scan.value.length);
        if (behind > 0) {
          scan.next(behind);
          scanPos = from;
        }
        for (let check = from; ;) {
          if (check >= to) {
            point = true;
            break;
          }
          if (!scan.lineBreak && scanPos + scan.value.length > check) break;
          check = scanPos + scan.value.length;
          scanPos += scan.value.length;
          scan.next();
        }
      }
    }
    const severity = worstSeverity(active);
    if (point) {
      builder.add(
        from,
        from,
        Decoration.widget({ widget: new DiagnosticPoint(severity), diagnostics: active.slice() }),
      );
    } else {
      const markClass = active.reduce((c, d) => {
        const extra = d.markClass;
        return extra ? `${c} ${extra}` : c;
      }, "");
      builder.add(
        from,
        to,
        Decoration.mark({
          class: `cm-lintRange cm-lintRange-${severity}${markClass}`,
          diagnostics: active.slice(),
          inclusiveEnd: active.some((a) => a.to > to),
        }),
      );
    }
    pos = to;
    if (pos === docLen) break;
    active = active.filter((a) => a.to > pos);
  }
  return builder.finish();
}

/** The worst severity of a group of diagnostics, as `@codemirror/lint` orders them. */
function worstSeverity(diagnostics: readonly LintDiagnostic[]): Severity {
  let worst: Severity = "hint";
  let weight = 1;
  for (const d of diagnostics) {
    const w = 4 - SEVERITY_RANK[d.severity];
    if (w > weight) {
      worst = d.severity;
      weight = w;
    }
  }
  return worst;
}
