// What the review screen remembers of a changeset while another is on view and across restarts: its
// open tabs, their view modes, where each pane was scrolled to and had its caret, and the hunk the
// Changes tab was at. `ViewMemory` keeps it per repository in the browser storage.

import { readStored, writeStored } from "../../lib/storage.ts";
import type { Mode, ViewMode } from "../../lib/viewMode.ts";
import type { HunkAnchor, PanePosition, ScrollPosition } from "./editor/ChangesetView.ts";
import { CHANGES, type TabId, commitOf, isFileTab, isStale, kindOf } from "./tabModel.ts";

/** The two panes of a file tab: its diff and its text each keep their own position. */
export interface FilePositions {
  diff?: PanePosition;
  text?: PanePosition;
}

export interface SavedView {
  open: TabId[];
  active: TabId;
  /** The view mode of each tab that has one. */
  modes: ReadonlyMap<TabId, ViewMode>;
  /** Where each file tab's panes were left. */
  panes: ReadonlyMap<TabId, FilePositions>;
  /** Where the Changes tab was scrolled to. */
  changes?: ScrollPosition;
  /** The current hunk of the Changes tab. */
  anchor?: HunkAnchor;
}

/**
 * The tabs a changeset comes back with: those it had that still have something to show (see
 * `isStale`; `changed` says which files the changeset changes; `hasLog` which logs are still
 * held), the Changes tab first, and the one that was active unless it is gone. A tab that stays
 * at a commit is kept: only the commit's own view can say whether it has anything left. A
 * changeset seen for the first time has the Changes tab alone, in the diff mode `diffMode`.
 */
export function restoredTabs(
  saved: SavedView | undefined,
  changed: (path: string) => boolean,
  diffMode: Mode,
  hasLog: (tab: TabId) => boolean,
) {
  const open = saved
    ? saved.open.filter((tab) => {
        if (kindOf(tab) === "log") return hasLog(tab);
        return commitOf(tab) !== undefined || !isStale(tab, saved.modes.get(tab), changed);
      })
    : [CHANGES];
  const active = saved && open.includes(saved.active) ? saved.active : CHANGES;
  const modes = new Map<TabId, ViewMode>();
  for (const tab of open) {
    const mode = saved?.modes.get(tab) ?? (tab === CHANGES ? diffMode : undefined);
    if (mode !== undefined) modes.set(tab, mode);
  }
  return { open, active, modes };
}

const VIEW_MODES: readonly unknown[] = ["unified", "split", "file"];

const isLine = (v: unknown): v is number => typeof v === "number" && Number.isInteger(v) && v >= 0;
const isTab = (v: unknown): v is TabId => v === CHANGES || (typeof v === "string" && isFileTab(v as TabId));
const record = (v: unknown): Record<string, unknown> | undefined =>
  typeof v === "object" && v !== null && !Array.isArray(v) ? (v as Record<string, unknown>) : undefined;

function decodePane(v: unknown): PanePosition | undefined {
  const r = record(v);
  return r && isLine(r.top) && isLine(r.caret) ? { top: r.top, caret: r.caret } : undefined;
}

function decodeScroll(v: unknown): ScrollPosition | undefined {
  const r = record(v);
  return r && typeof r.path === "string" && typeof r.offset === "number" && Number.isFinite(r.offset)
    ? { path: r.path, offset: r.offset }
    : undefined;
}

function decodeAnchor(v: unknown): HunkAnchor | undefined {
  const r = record(v);
  if (!r || typeof r.path !== "string" || !isLine(r.line)) return undefined;
  if (r.card === undefined) return { path: r.path, line: r.line };
  return typeof r.card === "string" ? { path: r.path, line: r.line, card: r.card as HunkAnchor["card"] } : undefined;
}

/** A stored view; none if the value is not one. Parts that are not valid are left out, not the whole. */
export function decodeView(value: unknown): SavedView | undefined {
  const r = record(value);
  if (!r || !Array.isArray(r.open)) return undefined;
  const open = [CHANGES, ...new Set(r.open.filter(isTab).filter((t) => t !== CHANGES))];
  const modes = new Map<TabId, ViewMode>();
  for (const [tab, mode] of Object.entries(record(r.modes) ?? {})) {
    if (open.includes(tab as TabId) && VIEW_MODES.includes(mode)) modes.set(tab as TabId, mode as ViewMode);
  }
  const panes = new Map<TabId, FilePositions>();
  for (const [tab, v] of Object.entries(record(r.panes) ?? {})) {
    const p = record(v);
    const diff = decodePane(p?.diff);
    const text = decodePane(p?.text);
    if (open.includes(tab as TabId) && (diff || text)) {
      panes.set(tab as TabId, { ...(diff && { diff }), ...(text && { text }) });
    }
  }
  const changes = decodeScroll(r.changes);
  const anchor = decodeAnchor(r.anchor);
  return {
    open,
    active: isTab(r.active) && open.includes(r.active) ? r.active : CHANGES,
    modes,
    panes,
    ...(changes && { changes }),
    ...(anchor && { anchor }),
  };
}

/** The view as stored: log tabs are not kept, as their content is fetched on demand. */
export function encodeView(view: SavedView): unknown {
  const open = view.open.filter((tab) => kindOf(tab) !== "log");
  const kept = <T>(map: ReadonlyMap<TabId, T>) => Object.fromEntries([...map].filter(([tab]) => open.includes(tab)));
  return {
    open,
    active: open.includes(view.active) ? view.active : CHANGES,
    modes: kept(view.modes),
    panes: kept(view.panes),
    changes: view.changes,
    anchor: view.anchor,
  };
}

/** The most changesets of a repository whose views are kept; the least recently left go first. */
export const MAX_REMEMBERED = 40;

export interface Storage {
  read(key: string): string | undefined;
  write(key: string, value: string | undefined): void;
}

const browserStorage: Storage = { read: readStored, write: writeStored };

/**
 * The views of one repository's changesets, by changeset key. The storage holds the
 * `MAX_REMEMBERED` most recently set and is read at each use, so a window never writes back what
 * another one has replaced; this window's own views (log tabs included) are kept in memory too.
 */
export class ViewMemory {
  private own = new Map<string, SavedView>();
  private readonly storageKey: string;
  private readonly storage: Storage;
  private readonly max: number;

  constructor(repo: string, storage: Storage = browserStorage, max = MAX_REMEMBERED) {
    this.storageKey = `vet.tabs:${repo}`;
    this.storage = storage;
    this.max = max;
  }

  /** The stored views, least recently set first. */
  private stored(): Map<string, SavedView> {
    const views = new Map<string, SavedView>();
    const text = this.storage.read(this.storageKey);
    if (text === undefined) return views;
    let entries: unknown;
    try {
      entries = JSON.parse(text);
    } catch {
      return views;
    }
    for (const entry of Array.isArray(entries) ? (entries as unknown[]) : []) {
      if (!Array.isArray(entry) || typeof entry[0] !== "string") continue;
      const view = decodeView(entry[1]);
      if (view) views.set(entry[0], view);
    }
    return views;
  }

  get(key: string): SavedView | undefined {
    return this.own.get(key) ?? this.stored().get(key);
  }

  set(key: string, view: SavedView) {
    this.own.set(key, view);
    const views = this.stored();
    views.delete(key);
    views.set(key, view);
    for (const oldest of views.keys()) {
      if (views.size <= this.max) break;
      views.delete(oldest);
    }
    const entries = [...views].map(([k, v]) => [k, encodeView(v)]);
    this.storage.write(this.storageKey, JSON.stringify(entries));
  }
}
