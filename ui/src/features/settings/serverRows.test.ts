import assert from "node:assert/strict";
import { test } from "node:test";
import type { ServerInfo } from "../../generated/ServerInfo.ts";
import type { Tsserver } from "../../generated/Tsserver.ts";
import { serverRow } from "./serverRows.ts";

const found = (origin: "custom" | "project" | "installed" | "path"): ServerInfo => ({
  language: "rust",
  title: "Rust",
  server: "rust-analyzer",
  state: { type: "found", path: "/bin/ra", origin, version: "1.2", tsserver: null },
  customPath: origin === "custom" ? "/bin/ra" : null,
  pinnedVersion: "2026-01-01",
  installBlocker: null,
  canInstall: true,
});

const missing: ServerInfo = {
  language: "python",
  title: "Python",
  server: "Pyright",
  state: { type: "missing", reason: "pyright-langserver is not on PATH" },
  customPath: null,
  pinnedVersion: "1.1.400",
  installBlocker: null,
  canInstall: true,
};

void test("a found server shows where it came from", () => {
  const row = serverRow(found("installed"));
  assert.equal(row.status, "found");
  assert.equal(row.origin, "Installed by Vet");
  assert.equal(row.path, "/bin/ra");
  assert.equal(row.version, "1.2");
});

void test("a custom origin is the custom status and keeps the path to reset", () => {
  const row = serverRow(found("custom"));
  assert.equal(row.status, "custom");
  assert.equal(row.customPath, "/bin/ra");
});

void test("the table names the source short, and a custom one by its path", () => {
  assert.equal(serverRow(found("installed")).source, "Vet");
  assert.equal(serverRow(found("path")).source, "PATH");
  assert.equal(serverRow(found("project")).source, "Project toolchain");
  assert.equal(serverRow(found("custom")).source, "/bin/ra");
  assert.equal(serverRow(missing).source, "");
});

void test("the tsserver of a found server is carried over, and a server without one has none", () => {
  const withTsserver = (tsserver: Tsserver): ServerInfo => {
    const info = found("path");
    if (info.state.type === "found") info.state.tsserver = tsserver;
    return info;
  };
  const using: Tsserver = { type: "using", whose: "the project's typescript 5.9.2", path: "/p/tsserver.js" };
  assert.deepEqual(serverRow(withTsserver(using)).tsserver, using);
  const none: Tsserver = { type: "missing", reason: "the project has no typescript" };
  assert.deepEqual(serverRow(withTsserver(none)).tsserver, none);
  assert.equal(serverRow(found("path")).tsserver, undefined);
});

void test("a missing server carries the reason and the backend's title and can be installed when the backend says so", () => {
  const row = serverRow(missing);
  assert.equal(row.status, "missing");
  assert.equal(row.problem, "pyright-langserver is not on PATH");
  assert.equal(row.title, "Python");
  assert.ok(row.canInstall);
});

void test("an install the backend cannot run stays off", () => {
  const blocked = serverRow({ ...missing, installBlocker: "npm is missing", canInstall: false });
  assert.equal(blocked.canInstall, false);
  assert.equal(blocked.installBlocker, "npm is missing");
});

void test("install progress overrides the resolved state", () => {
  assert.equal(serverRow(missing, { running: true }).status, "installing");
  assert.equal(serverRow(missing, { running: true }).canInstall, false);
  const failed = serverRow(missing, { running: false, error: "exit 1" });
  assert.equal(failed.status, "failed");
  assert.equal(failed.problem, "exit 1");
  assert.ok(failed.canInstall);
});
