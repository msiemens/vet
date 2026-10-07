// The semantic layer's answers for the views: hover, definition and diagnostics, with the
// diagnostics count and the "no language server" notes the footer shows (SPEC §7.1, I6). Every
// request carries the revision of the view that asks; a stale one is rejected by the backend and
// dropped here, as the views are rebuilt when the new revision arrives.

import { SvelteMap } from "svelte/reactivity";
import type { Diagnostic } from "../generated/Diagnostic";
import type { SemanticLocation } from "../generated/SemanticLocation";
import type { Side } from "../generated/Side";
import type { Unavailable } from "../generated/Unavailable";
import type { HoverText } from "./backend";
import { changeset } from "./changeset.svelte";
import { InFlight } from "./inFlight";
import { type SemanticSummary, summarize } from "./semanticSummary";
import { session } from "./session.svelte";

const NOTHING: HoverText = { text: "", code: [] };

class SemanticStore {
  private revision = -1;
  /** Bumped when the cache is dropped, so an answer to an earlier question is not stored. */
  private epoch = 0;
  private pending = new Map<string, Promise<Diagnostic[]>>();
  private hovering = new InFlight<HoverText>();
  /** The number of diagnostics to tell the viewer about, per file asked about at `revision`. */
  private counts = new SvelteMap<string, number>();
  /** The files the backend could not answer for, and why. */
  private missing = new SvelteMap<string, Unavailable>();

  summary(path?: string): SemanticSummary {
    return summarize(this.counts, this.missing, path);
  }

  /** The files of a new revision are asked afresh; `false` for a request of an older one. */
  private enter(revision: number): boolean {
    if (revision < this.revision) return false;
    if (revision > this.revision) {
      this.revision = revision;
      this.invalidate();
    }
    return true;
  }

  /** A repository was opened: its revisions are numbered afresh. */
  reset() {
    this.revision = -1;
    this.invalidate();
  }

  /** What the servers answered may differ now (a server was installed or its path changed). */
  invalidate() {
    this.epoch++;
    this.pending.clear();
    this.hovering.clear();
    this.counts.clear();
    this.missing.clear();
  }

  private note(revision: number, path: string, unavailable: Unavailable | undefined) {
    if (unavailable && this.enter(revision)) this.missing.set(path, unavailable);
  }

  /**
   * Markdown; the text is empty if the server has nothing to say or the request is stale. The same
   * position asked again while its answer is underway (an editor re-fires its hover when it updates)
   * joins that request.
   */
  hover(revision: number, side: Side, path: string, line: number, col: number): Promise<HoverText> {
    const backend = session.backend;
    if (!backend) return Promise.resolve(NOTHING);
    return this.hovering.share(JSON.stringify([revision, side, path, line, col]), async () => {
      try {
        const r = await backend.hover(revision, side, path, line, col);
        this.note(revision, path, r.unavailable);
        return { text: r.text, code: r.code };
      } catch {
        return NOTHING;
      }
    });
  }

  /** The definitions at the position, in `side`'s snapshot; `undefined` after telling the viewer why there are none. */
  async definition(
    revision: number,
    side: Side,
    path: string,
    line: number,
    col: number,
  ): Promise<SemanticLocation[] | undefined> {
    const backend = session.backend;
    if (!backend) return undefined;
    try {
      const r = await backend.definition(revision, side, path, line, col);
      this.note(revision, path, r.unavailable);
      if (r.unavailable) session.notice = r.unavailable.message;
      else if (!r.locations.length) session.notice = "No definition found";
      else return r.locations;
    } catch (error) {
      changeset.reportFailure(error, revision);
    }
    return undefined;
  }

  /** The target-side diagnostics of a file, asked once per revision; empty if unavailable or stale. */
  diagnostics(revision: number, path: string): Promise<Diagnostic[]> {
    if (!this.enter(revision)) return Promise.resolve([]);
    let answer = this.pending.get(path);
    if (!answer) {
      answer = this.fetch(revision, path);
      this.pending.set(path, answer);
    }
    return answer;
  }

  private async fetch(revision: number, path: string): Promise<Diagnostic[]> {
    const backend = session.backend;
    const epoch = this.epoch;
    try {
      const r = await backend?.diagnostics(revision, path);
      if (!r || epoch !== this.epoch) return [];
      if (r.unavailable) this.missing.set(path, r.unavailable);
      else {
        this.missing.delete(path);
        this.counts.set(path, r.count);
      }
      return r.diagnostics;
    } catch {
      if (epoch === this.epoch) this.pending.delete(path); // stale: the next revision asks again
      return [];
    }
  }
}

export const semantic = new SemanticStore();
