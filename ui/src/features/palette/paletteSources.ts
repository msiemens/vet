// Makes what Search Everywhere searches from what the window already holds: the changeset's files,
// the changed symbols the Changes tree lists, the changesets, the comment threads and the commands.
// The inputs are plain data, so the stores that hold it are read where the popup is drawn.

import type { ChangedSymbol } from "../../generated/ChangedSymbol";
import type { LanguageSymbols } from "../../generated/LanguageSymbols";
import type { ServerInfo } from "../../generated/ServerInfo";
import type { SymbolInfo } from "../../generated/SymbolInfo";
import type { Task } from "../../generated/Task";
import type { Thread } from "../../generated/Thread";
import { splitQualified } from "../../lib/qualified.ts";
import { shortLabel } from "../../lib/symbolLabel.ts";
import { fractionOf } from "../../state/taskHeadline.ts";
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

/** The files of the project the changeset does not change, from the list of all of them. */
export function unchangedCandidates(paths: readonly string[], changed: ReadonlySet<string>): FileCandidate[] {
  const out: FileCandidate[] = [];
  for (const path of paths) {
    if (!changed.has(path))
      out.push({ kind: "file", key: path, path, changed: false, added: 0, removed: 0, seen: 0, oldBase: 0 });
  }
  return out;
}

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

const symbolKey = (symbol: SymbolInfo) => `${symbol.id.path}#${symbol.id.name.join("\0")}`;

/**
 * The symbols the language servers found by name that the changeset did not change. One the changed
 * symbols already hold is theirs, listed once and as changed.
 */
export function serverSymbolCandidates(
  languages: readonly LanguageSymbols[],
  changed: readonly SymbolCandidate[],
): SymbolCandidate[] {
  const taken = new Set(changed.map((c) => symbolKey(c.info)));
  const out: SymbolCandidate[] = [];
  for (const { symbols } of languages) {
    for (const symbol of symbols) {
      const key = symbolKey(symbol);
      if (taken.has(key)) continue;
      taken.add(key);
      const label = shortLabel(symbol);
      out.push({
        kind: "symbol",
        key,
        info: symbol,
        symbolKind: symbol.kind,
        label,
        lead: splitQualified(label).qualifier.length,
        seen: 0,
        fromServer: true,
      });
    }
  }
  return out;
}

/** The languages whose server could not answer, with the server's name and why. */
export function failedServers(
  languages: readonly LanguageSymbols[],
  servers: readonly ServerInfo[],
): { language: string; server: string; failure: string }[] {
  return languages.flatMap((l) =>
    l.failure === null
      ? []
      : [
          {
            language: l.language,
            server: servers.find((s) => s.language === l.language)?.server ?? l.language,
            failure: l.failure,
          },
        ],
  );
}

/** A line for each answering language whose server is still indexing, which makes its answer incomplete. */
export function indexingNotes(
  languages: readonly LanguageSymbols[],
  servers: readonly ServerInfo[],
  tasks: readonly Task[],
): string[] {
  return languages.flatMap((l) => {
    const server = servers.find((s) => s.language === l.language);
    const indexing =
      server && l.failure === null && tasks.find((t) => t.kind === "serverIndex" && t.title.includes(server.server));
    if (!indexing) return [];
    const fraction = fractionOf(indexing);
    const how = fraction === undefined ? "" : ` (${Math.round(fraction * 100)}%)`;
    return [`${server.server} is still indexing${how}. Symbols of unchanged files may be incomplete.`];
  });
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
