import assert from "node:assert/strict";
import { test } from "node:test";
import type { ServerInfo } from "../generated/ServerInfo.ts";
import type { Task } from "../generated/Task.ts";
import type { Unavailable } from "../generated/Unavailable.ts";
import { summarize } from "./semanticSummary.ts";
import { hasServer, languageOf, serverLine, withoutFeature } from "./serverStatus.ts";

const server = (language: string, name: string, extensions: string[], fileNames: string[] = []): ServerInfo => ({
  language,
  title: language,
  server: name,
  extensions,
  fileNames,
  state: { type: "missing", reason: "" },
  customPath: null,
  pinnedVersion: null,
  installBlocker: null,
  canInstall: false,
  features: null,
});
const SERVERS = [server("rust", "rust-analyzer", ["rs"]), server("python", "Pyright", ["py", "pyi"])];

const task = (kind: Task["kind"], title: string, done: number | null = null, total: number | null = null): Task => ({
  project: 1,
  id: 1,
  kind,
  title,
  done,
  total,
  detail: null,
  state: { type: "running" },
});

const unavailable = (kind: Unavailable["kind"], language: string | null): Unavailable => ({
  kind,
  label: `label of ${kind}`,
  message: `why ${kind}`,
  language,
});

const quiet = summarize(new Map(), new Map());

void test("a file type has a server when one that serves its extension was found, running yet or not", () => {
  const found = (s: ServerInfo): ServerInfo => ({
    ...s,
    state: { type: "found", path: "/bin/x", origin: "path", version: null, tsserver: null },
  });
  const servers = [found(SERVERS[0]), SERVERS[1]];
  assert.equal(hasServer("src/a.rs", servers), true);
  assert.equal(hasServer("src/a.py", servers), false, "its server is not installed");
  assert.equal(hasServer("src/App.svelte", servers), false, "no server serves it");
  assert.equal(hasServer("Makefile", servers), false);
  assert.equal(hasServer("src/App.svelte", []), true, "the list is not loaded yet");
});

void test("the Changes tab names no server", () => {
  assert.equal(serverLine(undefined, SERVERS, [task("serverIndex", "Indexing rust-analyzer")], quiet), undefined);
});

void test("a file's server is found by its extension and is shown starting, then indexing with its progress", () => {
  assert.deepEqual(serverLine("src/a.rs", SERVERS, [task("serverStart", "Starting rust-analyzer")], quiet), {
    name: "rust-analyzer",
    state: "starting",
    title: "Starting rust-analyzer",
  });
  const indexing = task("serverIndex", "Indexing rust-analyzer for the base", 2, 8);
  assert.equal(serverLine("src/a.rs", SERVERS, [indexing], quiet)?.state, "indexing 25%");
  assert.equal(
    serverLine("src/a.rs", SERVERS, [task("serverIndex", "Indexing rust-analyzer")], quiet)?.state,
    "indexing",
  );
});

void test("another language's server running does not show for this file", () => {
  const line = serverLine("a.rs", SERVERS, [task("serverIndex", "Indexing Pyright", 1, 2)], quiet);
  assert.equal(line?.state, undefined);
});

void test("a server is ready once it has answered for the file", () => {
  const answered = summarize(new Map([["a.py", 0]]), new Map(), "a.py");
  assert.deepEqual(serverLine("a.py", SERVERS, [], answered), { name: "Pyright", state: "ready" });
  assert.deepEqual(serverLine("a.py", SERVERS, [], summarize(new Map(), new Map(), "a.py")), {
    name: "Pyright",
    state: undefined,
  });
});

void test("a server that is missing or failed is named with the backend's message", () => {
  const failed = summarize(new Map(), new Map([["a.rs", unavailable("serverFailed", "rust")]]), "a.rs");
  assert.deepEqual(serverLine("a.rs", SERVERS, [], failed), {
    name: "rust-analyzer",
    state: "failed",
    title: "why serverFailed",
  });
  const absent = summarize(new Map(), new Map([["a.py", unavailable("serverMissing", "python")]]), "a.py");
  assert.equal(serverLine("a.py", SERVERS, [], absent)?.state, "not installed");
});

void test("the language of a file is the one whose server serves its extension, whether or not that server is found", () => {
  assert.equal(languageOf("src/app.PY", SERVERS), "python");
  assert.equal(languageOf("lib/a.pyi", SERVERS), "python");
  assert.equal(languageOf("notes.md", SERVERS), undefined);
  assert.equal(languageOf("Makefile", SERVERS), undefined);
});

void test("a file without an extension is served by its name, also followed by a dot and anything", () => {
  const servers = [...SERVERS, server("dockerfile", "dockerfile-language-server", ["dockerfile"], ["Dockerfile"])];
  for (const path of ["Dockerfile", "svc/api/Dockerfile", "Dockerfile.dev", "api.dockerfile"]) {
    assert.equal(languageOf(path, servers), "dockerfile", path);
  }
  for (const path of ["Dockerfiles", "my-Dockerfile", "Dockerfile/readme"]) {
    assert.equal(languageOf(path, servers), undefined, path);
  }
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
    { ...server("rust", "rust-analyzer", ["rs"]), features: rust && { ...all, ...rust } },
    { ...server("python", "Pyright", ["py"]), features: python && { ...all, ...python } },
  ];
  const servers = says({}, { implementations: false, callHierarchy: false });
  assert.equal(withoutFeature("a.py", servers, "implementations"), "Pyright");
  assert.equal(withoutFeature("a.py", servers, "callHierarchy"), "Pyright");
  assert.equal(withoutFeature("a.py", servers, "hover"), undefined);
  assert.equal(withoutFeature("a.rs", servers, "implementations"), undefined);
  assert.equal(withoutFeature("a.rs", says({ hover: false }, null), "hover"), "rust-analyzer");
  assert.equal(withoutFeature("a.py", says(null, null), "hover"), undefined);
  assert.equal(withoutFeature("a.txt", servers, "implementations"), undefined);
  assert.equal(withoutFeature("a.py", [], "implementations"), undefined);
});

void test("a file without a server shows the backend's label alone", () => {
  const none = summarize(new Map(), new Map([["a.txt", unavailable("unsupported", null)]]), "a.txt");
  assert.deepEqual(serverLine("a.txt", SERVERS, [], none), { name: "label of unsupported", title: "why unsupported" });
  assert.equal(serverLine("a.txt", SERVERS, [], summarize(new Map(), new Map(), "a.txt")), undefined);
});
