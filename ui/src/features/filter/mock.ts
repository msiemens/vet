// Editing the filter is not in the protocol yet; the manifest only reports how many files it hides.

import type { ReviewFilter } from "./FilterPopover.svelte";

export const mockFilter: ReviewFilter = {
  roots: ["src", "tests"],
  hideBinary: false,
  include: [],
  exclude: ["docs/**", "**/*.lock"],
};
