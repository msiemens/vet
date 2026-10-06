// What the backend sent about the open changeset, plus the display state derived from it.

import type { FileEntry } from "../generated/FileEntry";
import { Backend } from "./backend";

export interface FileState {
  entry: FileEntry;
  seen: boolean[];
  /** Seen against an older base than the current one (amber). Mocked until the protocol carries it. */
  oldBase: boolean[];
}

/** Changed lines of a file in total, seen, and seen against an old base. */
export function lineCoverage(f: FileState) {
  let total = 0;
  let seen = 0;
  let old = 0;
  f.entry.hunks.forEach(([base, target], i) => {
    const lines = base + target;
    total += lines;
    if (f.oldBase[i]) old += lines;
    else if (f.seen[i]) seen += lines;
  });
  return { total, seen, old };
}

class ChangesetStore {
  status = $state("connecting…");
  backend = $state.raw<Backend>();
  name = $state("");
  files = $state.raw<FileState[]>([]);

  /** Share of changed lines seen, and seen against an old base, in percent. */
  coverage = $derived.by(() => {
    let total = 0;
    let seen = 0;
    let old = 0;
    for (const f of this.files) {
      const c = lineCoverage(f);
      total += c.total;
      seen += c.seen;
      old += c.old;
    }
    return { total, seen, old, seenPct: total ? (seen / total) * 100 : 0, oldPct: total ? (old / total) * 100 : 0 };
  });

  async connect() {
    try {
      this.backend = await Backend.connect();
    } catch (error) {
      this.status = (error as Error).message;
    }
  }

  open(name: string, files: FileState[]) {
    this.name = name;
    this.files = files;
  }

  update(path: string, seen: boolean[], oldBase: boolean[]) {
    this.files = this.files.map((f) => (f.entry.path === path ? { ...f, seen, oldBase } : f));
  }
}

export const changeset = new ChangesetStore();
