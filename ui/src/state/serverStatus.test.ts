import assert from "node:assert/strict";
import { test } from "node:test";
import type { ServerInfo } from "../generated/ServerInfo.ts";
import type { ServerPhase } from "../generated/ServerPhase.ts";
import type { ServerRuntimes } from "../generated/ServerRuntimes.ts";
import type { Serving } from "../generated/Serving.ts";
import type { Unavailable } from "../generated/Unavailable.ts";
import { summarize } from "./semanticSummary.ts";
import { atWork, hasServer, indexingOf, serverLine, withoutFeature } from "./serverStatus.ts";

const server = (language: string, name: string): ServerInfo => ({
  language,
  title: language,
  server: name,
  state: { type: "missing", reason: "" },
  customPath: null,
  pinnedVersion: null,
  installBlocker: null,
  canInstall: false,
  features: null,
});
const SERVERS = [server("rust", "rust-analyzer"), server("python", "Pyright")];

const unavailable = (kind: Unavailable["kind"], language: string | null): Unavailable => ({
  kind,
  label: `label of ${kind}`,
  message: `why ${kind}`,
  language,
});

const serving = (language: string): Serving => ({ type: "language", language });
const runtimes = (...rows: [string, "target" | "base", ServerPhase][]): ServerRuntimes => ({
  serial: 1,
  runtimes: rows.map(([language, side, phase]) => ({ language, side, phase })),
});
const indexing = (done: number | null, total: number | null, detail: string | null = null): ServerPhase => ({
  type: "indexing",
  done,
  total,
  detail,
});

const quiet = summarize(new Map(), new Map());
const NONE = runtimes();

void test("a file has a server when the one that serves it was found, running yet or not", () => {
  const found = (s: ServerInfo): ServerInfo => ({
    ...s,
    state: { type: "found", path: "/bin/x", origin: "path", version: null, tsserver: null },
  });
  const servers = [found(SERVERS[0]), SERVERS[1]];
  assert.equal(hasServer(serving("rust"), servers), true);
  assert.equal(hasServer(serving("python"), servers), false, "its server is not installed");
  assert.equal(hasServer({ type: "unsupported", why: unavailable("unsupported", null) }, servers), false);
  assert.equal(hasServer(undefined, servers), true, "it is not known who serves the file yet");
  assert.equal(hasServer(serving("rust"), []), true, "the list is not loaded yet");
});

void test("the Changes tab names no server", () => {
  const busy = runtimes(["rust", "target", { type: "starting" }]);
  assert.equal(serverLine(undefined, undefined, SERVERS, busy, quiet), undefined);
});

void test("a file's server is shown starting, then indexing with its progress", () => {
  const line = (r: ServerRuntimes) => serverLine("src/a.rs", serving("rust"), SERVERS, r, quiet);
  assert.deepEqual(line(runtimes(["rust", "target", { type: "starting" }])), {
    name: "rust-analyzer",
    state: "starting",
    title: "Starting rust-analyzer",
  });
  assert.deepEqual(
    line(runtimes(["rust", "base", { type: "starting" }]))?.title,
    "Starting rust-analyzer for the base",
  );
  const quarter = line(runtimes(["rust", "target", indexing(2, 8, "Roots Scanned")]));
  assert.deepEqual(quarter, {
    name: "rust-analyzer",
    state: "indexing 25%",
    title: "Roots Scanned",
    indexing: { fraction: 0.25 },
  });
  assert.equal(line(runtimes(["rust", "target", indexing(null, null)]))?.state, "indexing");
});

void test("work in progress is said before readiness, and the target side before the base", () => {
  const line = serverLine(
    "a.rs",
    serving("rust"),
    SERVERS,
    runtimes(["rust", "base", indexing(1, 2)], ["rust", "target", { type: "ready" }]),
    quiet,
  );
  assert.equal(line?.state, "indexing 50%");
  assert.deepEqual(indexingOf("rust", runtimes(["rust", "base", indexing(1, 4)], ["rust", "target", indexing(3, 4)])), {
    fraction: 0.75,
    detail: undefined,
  });
  assert.equal(indexingOf("rust", runtimes(["rust", "target", { type: "ready" }])), undefined);
});

void test("another language's server running does not show for this file", () => {
  const line = serverLine("a.rs", serving("rust"), SERVERS, runtimes(["python", "target", indexing(1, 2)]), quiet);
  assert.equal(line?.state, undefined);
  assert.equal(atWork("rust", runtimes(["python", "target", { type: "starting" }])), false);
  assert.equal(atWork("python", runtimes(["python", "base", { type: "starting" }])), true);
  assert.equal(atWork("python", runtimes(["python", "base", { type: "ready" }])), false);
  assert.equal(atWork(undefined, runtimes(["python", "base", { type: "starting" }])), false);
});

void test("a server is ready as the backend says, not once it has answered", () => {
  const ready = runtimes(["python", "target", { type: "ready" }]);
  assert.deepEqual(serverLine("a.py", serving("python"), SERVERS, ready, quiet), { name: "Pyright", state: "ready" });
  assert.deepEqual(serverLine("a.py", serving("python"), SERVERS, NONE, quiet), { name: "Pyright" });
});

void test("a server that is missing or failed is named with the backend's message", () => {
  const failed = summarize(new Map(), new Map([["a.rs", unavailable("serverFailed", "rust")]]), "a.rs");
  assert.deepEqual(serverLine("a.rs", serving("rust"), SERVERS, NONE, failed), {
    name: "rust-analyzer",
    state: "failed",
    title: "why serverFailed",
  });
  const absent = runtimes(["python", "target", { type: "failed", why: unavailable("serverMissing", "python") }]);
  assert.deepEqual(serverLine("a.py", serving("python"), SERVERS, absent, quiet), {
    name: "Pyright",
    state: "not installed",
    title: "why serverMissing",
  });
  const limited = runtimes(["rust", "target", { type: "failed", why: unavailable("analysisLimited", "rust") }]);
  assert.deepEqual(serverLine("a.rs", serving("rust"), SERVERS, limited, quiet), {
    name: "label of analysisLimited",
    title: "why analysisLimited",
  });
});

void test("a server is without a feature only once it has said so, whatever the language", () => {
  const all = {
    documentSymbols: true,
    definition: true,
    references: true,
    implementations: true,
    hover: true,
    callHierarchy: true,
    workspaceSymbols: true,
  };
  const says = (rust: Partial<typeof all> | null, python: Partial<typeof all> | null) => [
    { ...server("rust", "rust-analyzer"), features: rust && { ...all, ...rust } },
    { ...server("python", "Pyright"), features: python && { ...all, ...python } },
  ];
  const servers = says({}, { implementations: false, callHierarchy: false });
  assert.equal(withoutFeature("python", servers, "implementations"), "Pyright");
  assert.equal(withoutFeature("python", servers, "callHierarchy"), "Pyright");
  assert.equal(withoutFeature("python", servers, "hover"), undefined);
  assert.equal(withoutFeature("rust", servers, "implementations"), undefined);
  assert.equal(withoutFeature("rust", says({ hover: false }, null), "hover"), "rust-analyzer");
  assert.equal(withoutFeature("python", says(null, null), "hover"), undefined);
  assert.equal(withoutFeature(undefined, servers, "implementations"), undefined);
  assert.equal(withoutFeature("python", [], "implementations"), undefined);
});

void test("a file without a server shows the backend's label alone", () => {
  const none = summarize(new Map(), new Map([["a.txt", unavailable("unsupported", null)]]), "a.txt");
  assert.deepEqual(serverLine("a.txt", undefined, SERVERS, NONE, none), {
    name: "label of unsupported",
    title: "why unsupported",
  });
  assert.equal(serverLine("a.txt", undefined, SERVERS, NONE, summarize(new Map(), new Map(), "a.txt")), undefined);
});
