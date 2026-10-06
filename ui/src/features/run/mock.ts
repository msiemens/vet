// Run targets will be read from manifests by the backend (SPEC §10).

import type { RunTarget } from "./RunButton.svelte";

export const mockTargets: RunTarget[] = [
  { id: "sync-worker", name: "sync-worker", source: "cargo bin" },
  { id: "migrate", name: "migrate", source: "cargo bin" },
  { id: "test", name: "test", source: "package.json" },
];
