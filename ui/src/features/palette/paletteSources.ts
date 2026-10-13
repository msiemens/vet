// Makes what Search Everywhere searches from what the window already holds: the changeset's files,
// the changed symbols the Changes tree lists, the changesets, the comment threads and the commands.
// The inputs are plain data, so the stores that hold it are read where the popup is drawn.

import type { ChangedSymbol } from "../../generated/ChangedSymbol";
import type { Thread } from "../../generated/Thread";
import { splitQualified } from "../../lib/qualified.ts";
import { shortLabel } from "../../lib/symbolLabel.ts";
import type {
  ActionCandidate,
  ChangesetCandidate,
  FileCandidate,
  SymbolCandidate,
  ThreadCandidate,
} from "./paletteRows.ts";

/** A changed file as the changeset's manifest and review state say. */
export interface ChangedFile {
  path: string;
  status: "M" | "A" | "D" | "R";
  added: number;
  removed: number;
  seen: number;
  oldBase: number;
}

export const fileCandidates = (files: readonly ChangedFile[]): FileCandidate[] =>
  files.map((f) => ({ kind: "file", key: f.path, changed: true, ...f }));

/** The changed symbols of each file, as a symbol is chosen by the Changes tree, with how much of its file is seen. */
export function symbolCandidates(
  files: Iterable<{ symbols: readonly ChangedSymbol[]; seen: number }>,
): SymbolCandidate[] {
  const out: SymbolCandidate[] = [];
  for (const { symbols, seen } of files) {
    for (const { symbol, change } of symbols) {
      const label = shortLabel(symbol);
      out.push({
        kind: "symbol",
        key: `${symbol.id.path}#${symbol.id.name.join("\0")}${change}`,
        info: symbol,
        symbolKind: symbol.kind,
        label,
        lead: splitQualified(label).qualifier.length,
        change,
        seen,
      });
    }
  }
  return out;
}

/** A changeset as the switcher lists it. */
export interface ChangesetEntry {
  id: string;
  number?: string;
  title: string;
  source: string;
  github: boolean;
  onView: boolean;
  /** `finished` also says whether it was complete. */
  progress: { kind: "none" } | { kind: "started" } | { kind: "progress"; seen: number } | { kind: "finished" };
}

export const changesetCandidates = (entries: readonly ChangesetEntry[]): ChangesetCandidate[] =>
  entries.map((e) => ({
    kind: "changeset",
    key: e.id,
    number: e.number,
    title: e.title,
    source: e.source,
    icon: e.github ? "github" : "branch",
    onView: e.onView,
    seen: e.progress.kind === "progress" ? e.progress.seen : e.progress.kind === "finished" ? 100 : undefined,
    finished: e.progress.kind === "finished",
  }));

/** Threads with a place: a file's or line's by its path and first line, a changeset's by its name. */
export function threadCandidates(threads: readonly Thread[]): ThreadCandidate[] {
  return threads.map((t) => {
    const line = t.location ? `:${t.location.start + 1}` : "";
    return {
      kind: "thread",
      key: String(t.id),
      text: t.messages[0]?.body.replace(/\s+/g, " ").trim() ?? "",
      where: t.path === null ? "changeset" : `${t.path}${line}`,
      resolved: t.status === "resolved",
      moved: t.anchor?.type === "inferred" || t.anchor?.type === "ambiguous",
    };
  });
}

/** A registered command with what the registry says of it in a context. */
export interface CommandEntry {
  id: string;
  label: string;
  binding?: string;
  enabled: boolean;
  reason?: string;
}

/** The commands a key can run, but not the popup's own. */
export function actionCandidates(commands: readonly CommandEntry[], skip: (id: string) => boolean): ActionCandidate[] {
  return commands
    .filter((c) => !skip(c.id))
    .map((c) => ({
      kind: "action",
      key: c.id,
      label: c.label,
      binding: c.binding,
      disabled: c.reason,
      off: !c.enabled,
    }));
}
