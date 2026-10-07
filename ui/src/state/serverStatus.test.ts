import assert from "node:assert/strict";
import { test } from "node:test";
import type { ServerInfo } from "../generated/ServerInfo.ts";
import type { Task } from "../generated/Task.ts";
import type { Unavailable } from "../generated/Unavailable.ts";
import { summarize } from "./semanticSummary.ts";
import { serverLine } from "./serverStatus.ts";

const server = (language: string, name: string, extensions: string[]): ServerInfo => ({
  language,
  title: language,
  server: name,
  extensions,
  state: { type: "missing", reason: "" },
  customPath: null,
  pinnedVersion: null,
  installBlocker: null,
  canInstall: false,
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

void test("a file without a server shows the backend's label alone", () => {
  const none = summarize(new Map(), new Map([["a.txt", unavailable("unsupported", null)]]), "a.txt");
  assert.deepEqual(serverLine("a.txt", SERVERS, [], none), { name: "label of unsupported", title: "why unsupported" });
  assert.equal(serverLine("a.txt", SERVERS, [], summarize(new Map(), new Map(), "a.txt")), undefined);
});
