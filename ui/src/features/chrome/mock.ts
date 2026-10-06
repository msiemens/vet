// Until protocol v1 reports the session, the snapshot and the language servers.

import type { FooterInfo } from "./StatusFooter.svelte";
import type { HeaderInfo } from "./Header.svelte";

export const mockHeader: HeaderInfo = {
  project: "ledger-sync",
  changeset: { label: "Working tree", branch: "main" },
  base: "a41c9e2+",
  target: { label: "#17", settled: true },
};

export const mockFooter: FooterInfo = {
  snapshot: "#17 · 12 s",
  languageServer: "rust-analyzer",
  topology: { label: "native", ok: true },
  cursor: "44:9",
  eol: "LF",
  encoding: "UTF-8",
  diagnostics: 0,
};
