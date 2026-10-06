// Placeholder until the semantic layer (SPEC §7) answers queries.

import type { SymbolInfo } from "./SymbolPanel.svelte";

export const mockSymbol: SymbolInfo = {
  name: "next_delay",
  qualified: "ledger_sync::sync::retry::RetryPolicy",
  signature: "pub fn next_delay(&self, attempt: u32) -> Duration",
  doc: "Delay before retry number attempt, starting at 0.",
  callers: [
    { name: "JobRunner::attempt", at: "job.rs:118" },
    { name: "Client::send", at: "client.rs:92" },
    { name: "caps_delay", at: "retry_policy.rs:14" },
  ],
  reachedFrom: ["main", "SyncWorker::run", "JobRunner::attempt", "next_delay"],
  removed: {
    name: "backoff_ms",
    sites: [
      { at: "client.rs:88", verdict: "unresolved" },
      { at: "legacy.rs:22", verdict: "binding changed", detail: "util::backoff_ms" },
      { at: "job.rs:131", verdict: "gone" },
      { at: "retry.rs:15", verdict: "unknown" },
    ],
  },
};
