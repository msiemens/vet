// Stand-ins for what the protocol does not carry yet: hunk symbols (semantic layer)
// and the executor-relevant file list (backend configuration, SPEC §7.5).

const hash = (s: string) => [...s].reduce((h, c) => (h * 31 + c.charCodeAt(0)) >>> 0, 7);

const SYMBOLS = ["impl RetryPolicy", "fn next_delay", "fn should_retry", "struct Config", "mod tests"];

export function mockSymbols(path: string, hunks: number): (string | undefined)[] {
  const offset = hash(path);
  return Array.from({ length: hunks }, (_, i) => (i % 4 === 3 ? undefined : SYMBOLS[(offset + i) % SYMBOLS.length]));
}

export const mockIsBuildFile = (path: string) => /(^|\/)(Cargo\.toml|build\.rs|package\.json|pyproject\.toml|setup\.py|[^/]+\.csproj)$/.test(path);
