// A line of text that was found (a usage, a search match) as a row shows it: cut where its syntax
// colour changes or a match starts or ends.

import type { LocationRow } from "../generated/LocationRow.ts";
import type { SearchHit } from "../generated/SearchHit.ts";
import type { SearchLine } from "../generated/SearchLine.ts";
import { decodeHighlights, eachRange, type Highlights, pieces } from "./highlights.ts";

/** A stretch of a row's line: coloured as the syntax has it, and `hit` where the match is. */
export interface CodeRun {
  text: string;
  className?: string;
  hit: boolean;
}

/** The row's line cut where its syntax colour changes or the match starts or ends. */
export function codeRuns(row: Pick<LocationRow, "text" | "highlights" | "matchStart" | "matchEnd">): CodeRun[] {
  const { text, matchStart, matchEnd } = row;
  const spans: { from: number; to: number; className?: string }[] = [];
  let at = 0;
  eachRange(decodeHighlights(row.highlights), 0, text.length, (from, to, className) => {
    if (from > at) spans.push({ from: at, to: from });
    spans.push({ from, to, className });
    at = to;
  });
  if (at < text.length) spans.push({ from: at, to: text.length });

  const runs: CodeRun[] = [];
  for (const span of spans) {
    const cuts = [span.from, ...[matchStart, matchEnd].filter((c) => c > span.from && c < span.to), span.to];
    for (let i = 1; i < cuts.length; i++) {
      const [from, to] = [cuts[i - 1], cuts[i]];
      runs.push({
        text: text.slice(from, to),
        className: span.className,
        hit: matchStart < matchEnd && from >= matchStart && to <= matchEnd,
      });
    }
  }
  return runs;
}

/** A library's file: the package it belongs to, if the path tells, and the rest of the path. */
export function libraryLabel(path: string): { package?: string; rest: string } {
  const slashed = path.replaceAll("\\", "/");
  const crate = /\/registry\/src\/[^/]+\/(.+)-(\d[^/]*)\/(.+)$/.exec(slashed);
  if (crate) return { package: `${crate[1]} ${crate[2]}`, rest: crate[3] };
  const module = /\/node_modules\/((?:@[^/]+\/)?[^/]+)\/(.+)$/.exec(slashed);
  if (module) return { package: module[1], rest: module[2] };
  return { rest: path };
}

/** What the line starts with is left out of its row; the matches' columns move with it. */
export function trimmed(line: Pick<SearchLine, "text" | "textStart" | "hits">): { text: string; hits: SearchHit[] } {
  const lead = line.text.length - line.text.trimStart().length;
  return {
    text: line.text.slice(lead),
    hits: line.hits
      .map((h) => ({ from: h.from - line.textStart - lead, to: h.to - line.textStart - lead }))
      .filter((h) => h.to > 0)
      .map((h) => ({ from: Math.max(0, h.from), to: h.to })),
  };
}

/** A stretch of a line with its syntax colour and whether a match covers it. */
export interface Segment {
  text: string;
  className?: string;
  find?: "hit" | "current";
}

/**
 * `text`, which starts at `start` in the text `highlights` colour, cut where the colour or a match
 * changes. `hits` are columns of `text`; the one at `current` is the result being looked at.
 */
export function segments(
  text: string,
  start: number,
  highlights: Highlights,
  hits: readonly SearchHit[],
  current?: number,
): Segment[] {
  const out: Segment[] = [];
  let at = 0;
  for (const piece of pieces(highlights, start, text)) {
    const [chunk, className] = typeof piece === "string" ? [piece, undefined] : [piece.text, piece.className];
    const end = at + chunk.length;
    let cut = at;
    // The matches that overlap this piece, in order, cut it up.
    for (const [i, h] of hits.entries()) {
      if (h.to <= cut || h.from >= end) continue;
      const from = Math.max(h.from, cut);
      const to = Math.min(h.to, end);
      if (from > cut) out.push({ text: text.slice(cut, from), className });
      out.push({ text: text.slice(from, to), className, find: i === current ? "current" : "hit" });
      cut = to;
    }
    if (cut < end) out.push({ text: text.slice(cut, end), className });
    at = end;
  }
  return out;
}
