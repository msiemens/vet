// Invariant I8: what the editor views show of the review state is what the backend says. The views are
// read through `window.__vet.rendered()` in a real browser; the backend's side is a fresh manifest of the
// view the page shows.

import type { Manifest } from "../../src/generated/Manifest.ts";
import type { Rendered, RenderedFile, RenderedView } from "../../src/features/review/editor/rendered.ts";
import { countText } from "../../src/features/review/editor/fileHeader.ts";
import { markKind } from "../../src/features/review/editor/scrollMarks.ts";

export interface Difference {
  what: string;
  page: unknown;
  backend: unknown;
}

const same = (a: unknown, b: unknown) => JSON.stringify(a) === JSON.stringify(b);

/** What a diff view of the manifest's files (minus the ones in `skip`) should show. */
export function expectedView(manifest: Manifest): RenderedView {
  const files = manifest.files.map((f): RenderedFile => ({
    path: f.path,
    count: countText(f.completion),
    seenPercent: f.completion.seenPercent,
    allSeen: f.completion.complete,
    reconfirm: f.review.hunks.includes("seenAgainstOldBase"),
    cards: f.nonText.map((item) => f.review.acknowledged.includes(item.kind)),
    hunks: f.review.hunks,
    flagged: f.review.flagged,
  }));
  const marks = manifest.files.flatMap((f) =>
    f.hunks.map((h, i) => markKind(f.review.hunks[i], h.targetLen === 0, f.review.flagged[i])),
  );
  return { files, marks };
}

/**
 * The differences between a view as drawn and as the manifest says it should be. An editor that is not
 * mounted draws no hunks, so only the files that have one are compared on those.
 */
export function compareView(drawn: RenderedView | undefined, manifest: Manifest): Difference[] {
  if (!drawn) return [{ what: "the view exists", page: null, backend: "a view" }];
  const expected = expectedView(manifest);
  const found: Difference[] = [];
  const differ = (what: string, page: unknown, backend: unknown) => {
    if (!same(page, backend)) found.push({ what, page, backend });
  };
  differ(
    "the files shown",
    drawn.files.map((f) => f.path),
    expected.files.map((f) => f.path),
  );
  for (const want of expected.files) {
    const got = drawn.files.find((f) => f.path === want.path);
    if (!got) continue;
    differ(`${want.path}: progress text`, got.count, want.count);
    differ(`${want.path}: meter`, Math.round(got.seenPercent), Math.round(want.seenPercent));
    differ(`${want.path}: all seen`, got.allSeen, want.allSeen);
    differ(`${want.path}: re-confirm offered`, got.reconfirm, want.reconfirm);
    differ(`${want.path}: cards`, got.cards, want.cards);
    if (got.hunks) differ(`${want.path}: hunk states in the editor`, got.hunks, want.hunks);
    if (got.flagged) differ(`${want.path}: hunk flags in the editor`, got.flagged, want.flagged);
  }
  differ("scroll bar marks", drawn.marks, expected.marks);
  return found;
}

/** The differences between the text view of `path` as drawn and the manifest. */
export function compareText(
  drawn: Rendered["texts"][string] | undefined,
  path: string,
  manifest: Manifest,
): Difference[] {
  const file = manifest.files.find((f) => f.path === path);
  if (!drawn?.hunks || !file) return [];
  const found: Difference[] = [];
  if (!same(drawn.hunks, file.review.hunks))
    found.push({ what: `${path}: hunk states in the text`, page: drawn.hunks, backend: file.review.hunks });
  if (!same(drawn.flagged, file.review.flagged))
    found.push({ what: `${path}: hunk flags in the text`, page: drawn.flagged, backend: file.review.flagged });
  return found;
}
