// What the dev tools read from a `ChangesetView`. A view hands this out only to an owner that asks
// for it (the benchmark); production code never does.

import type { Section } from "./section";
import type { SectionHeights } from "./sectionHeights";
import type { Stops } from "./stops";

export interface ViewProbe {
  readonly sections: readonly Section[];
  /** Height of the whole column. */
  totalHeight(): number;
  /** The number of stops (cards and hunks) hunk navigation moves over. */
  readonly stopCount: number;
  /** The stop of file `fi`'s first hunk. */
  firstHunkOf(fi: number): number;
  /** The button on the header of file `fi` that toggles everything in it. */
  seenButton(fi: number): HTMLButtonElement;
  /** Largest difference between a mounted section's real height and its estimate; expanded regions are accounted for. */
  maxDrift(): number;
  /** Mounted files whose merge chunk count differs from the backend's hunk count. */
  chunkMismatches(): number;
}

export function viewProbe(parts: {
  sections: readonly Section[];
  mounted: ReadonlySet<number>;
  heights: SectionHeights;
  stops: Stops;
}): ViewProbe {
  const { sections, mounted, heights, stops } = parts;
  return {
    sections,
    stopCount: stops.total,
    totalHeight: () => heights.total,
    firstHunkOf: (fi) => stops.start(fi) + sections[fi].cards.length,
    seenButton(fi) {
      const button = sections[fi].el.querySelector<HTMLButtonElement>("header .seen");
      if (!button) throw new Error(`file ${fi} has no Seen button`);
      return button;
    },
    maxDrift() {
      let drift = 0;
      for (const fi of mounted) {
        const s = sections[fi];
        const expected = heights.m.header + s.cardsHeight + s.block + heights.m.gap + 1;
        drift = Math.max(drift, Math.abs(s.body.scrollHeight + expected - s.height));
      }
      return drift;
    },
    chunkMismatches() {
      let n = 0;
      for (const fi of mounted) {
        const { handle, file } = sections[fi];
        if (handle && handle.chunkCount() !== file.hunks.length) n++;
      }
      return n;
    },
  };
}
