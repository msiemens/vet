// The filter is stored per session by the backend (SPEC §5.2, §10).

import type { ReviewFilter } from "./FilterPopover.svelte";

export const mockFilter: ReviewFilter = {
  roots: ["src", "tests"],
  hideBinary: false,
  include: [],
  exclude: ["docs/**", "**/*.lock"],
};

export const mockExcludedCount = 2;
