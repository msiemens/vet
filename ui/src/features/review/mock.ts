// Stand-ins for what protocol v1 will deliver (A.1): old-base state, hunk symbols,
// file modes, executor-relevant flags and non-text changes. Replace with real data in one place.

import type { FileEntry } from "../../generated/FileEntry";
import type { NonTextChange } from "./ChangeCard.svelte";

const hash = (s: string) => [...s].reduce((h, c) => (h * 31 + c.charCodeAt(0)) >>> 0, 7);

/** Marks some hunks as seen against an old base. */
export function mockOldBase(entry: FileEntry): boolean[] {
  return entry.hunks.map((_, i) => hash(entry.path) % 4 === 0 && i % 3 === 1);
}

const SYMBOLS = ["impl RetryPolicy", "fn next_delay", "fn should_retry", "struct Config", "mod tests"];

export function mockSymbols(path: string, hunks: number): (string | undefined)[] {
  const offset = hash(path);
  return Array.from({ length: hunks }, (_, i) => (i % 4 === 3 ? undefined : SYMBOLS[(offset + i) % SYMBOLS.length]));
}

/** The executor-relevant file list is configurable and lives in the backend (SPEC §7.5). */
export const mockIsBuildFile = (path: string) => /(^|\/)(Cargo\.toml|build\.rs|package\.json|pyproject\.toml|setup\.py|[^/]+\.csproj)$/.test(path);

export const mockIsExecutableChange = (path: string) => path.endsWith(".sh");

export const mockNonText: NonTextChange[] = [
  { path: "scripts/release.sh", kind: "mode", before: "100644", after: "100755" },
  { path: "assets/logo.png", kind: "binary", before: "4.2 KB", after: "5.8 KB", preview: true },
  { path: "assets/current", kind: "symlink", before: "releases/2024-05", after: "releases/2024-06" },
  { path: "vendor/zstd", kind: "submodule", before: "a41c9e2", after: "0f3b7d1" },
  { path: "data/dump.bin", kind: "unavailable", before: "", after: "", reason: "Larger than the capture limit (64 MB)" },
];
